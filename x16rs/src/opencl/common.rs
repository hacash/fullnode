use std::ffi::CString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

use ocl::enums::{ProgramInfo, ProgramInfoResult};
use ocl::{Buffer, Context, Device, EventList, Kernel, Platform, Program, Queue};

use super::HASH_WIDTH;

#[allow(dead_code)]
pub struct OpenCLResources {
    pub program: Program,
    pub queue: Queue,
    pub buffer_best_nonces: Buffer<u32>,
    pub buffer_best_nonces_diamond: Buffer<u64>,
    pub buffer_global_hashes: Buffer<u8>,
    pub buffer_global_order: Buffer<u32>,
    pub buffer_best_hashes: Buffer<u8>,
}

pub fn initialize_opencl(
    diamond_mining: bool,
    opencldir: &str,
    platformid: u32,
    deviceids: &str,
    workgroups: u32,
    localsize: u32,
    unitsize: u32,
) -> Vec<OpenCLResources> {
    if localsize != 256 {
        eprintln!(
            "[warn] OpenCL local_size={} is incompatible with kernel fixed local arrays(256); fallback to CPU miner.",
            localsize
        );
        return Vec::new();
    }

    let kernel_file = if diamond_mining {
        format!("{}x16rs_diamond.cl", opencldir)
    } else {
        format!("{}x16rs_main.cl", opencldir)
    };
    let kernel_path = Path::new(&kernel_file);

    let platforms = Platform::list();
    let Some(platform) = platforms.get(platformid as usize).copied() else {
        eprintln!(
            "[warn] OpenCL platform id {} is invalid; fallback to CPU miner.",
            platformid
        );
        return Vec::new();
    };

    println!("Platform name: {}", platform.name().unwrap_or_default());
    println!("Manufacturer: {}", platform.vendor().unwrap_or_default());
    println!("Version: {}", platform.version().unwrap_or_default());

    let mut cnf_devices: Vec<u32> = deviceids
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .filter_map(|s| s.trim().parse::<u32>().ok())
        .collect();

    if cnf_devices.is_empty() {
        match Device::list_all(platform) {
            Ok(platform_devices) => {
                for (idx, _) in platform_devices.iter().enumerate() {
                    cnf_devices.push(idx as u32);
                }
            }
            Err(e) => {
                eprintln!(
                    "[warn] cannot list OpenCL devices: {}; fallback to CPU miner.",
                    e
                );
                return Vec::new();
            }
        }
    }

    let mut devices = Vec::new();
    for &device_id in &cnf_devices {
        match Device::by_idx_wrap(platform, device_id as usize) {
            Ok(device) => devices.push(device),
            Err(e) => eprintln!("[warn] cannot find OpenCL device {}: {}", device_id, e),
        }
    }

    let global_work_size = workgroups.saturating_mul(localsize);
    let mut opencl_resource_devices = Vec::with_capacity(devices.len());
    for (idx, &device) in devices.iter().enumerate() {
        println!("-----------------------------------------");
        println!(
            "Device {}: {}",
            cnf_devices[idx],
            device.name().unwrap_or_default()
        );
        println!("-----------------------------------------");

        let context = Context::builder()
            .platform(platform)
            .devices(device)
            .build()
            .expect("can't create OpenCL context");

        if !Path::new(opencldir).is_dir() {
            eprintln!(
                "[warn] OpenCL dir not found: {}; fallback to CPU miner.",
                opencldir
            );
            return Vec::new();
        }

        let device_name = device.name().unwrap_or_else(|_| "device".to_owned());
        let binary_file = format!(
            "{}{}_{}{}.bin",
            opencldir,
            device_name,
            cnf_devices[idx],
            if diamond_mining { "_diamonds" } else { "" }
        );
        let binary_path = Path::new(&binary_file);

        let need_recompile = if binary_path.exists() {
            let binary_modified = fs::metadata(binary_path).and_then(|meta| meta.modified());
            let kernel_modified = fs::metadata(kernel_path).and_then(|meta| meta.modified());
            match (binary_modified, kernel_modified) {
                (Ok(binary_modified), Ok(kernel_modified)) => kernel_modified > binary_modified,
                _ => true,
            }
        } else {
            true
        };

        let program = if !need_recompile {
            let mut binary_file = File::open(binary_path).expect("cannot open OpenCL binary file");
            let mut binary_data = Vec::new();
            binary_file
                .read_to_end(&mut binary_data)
                .expect("cannot read OpenCL binary file");
            println!("Loading OpenCL from the binary...");
            let binaries = [&binary_data[..]];
            Program::with_binary(&context, &[device], &binaries, &CString::new("").unwrap())
                .expect("can't create OpenCL program with the binary file")
        } else {
            println!("Compiling OpenCL kernel...");
            compile_program_from_source(&context, &device, kernel_path, binary_path, opencldir)
        };

        let queue = Queue::new(&context, device, None).expect("can't create OpenCL event queue");
        opencl_resource_devices.push(OpenCLResources {
            program: program.clone(),
            queue: queue.clone(),
            buffer_best_nonces: Buffer::<u32>::builder()
                .queue(queue.clone())
                .flags(ocl::core::MEM_WRITE_ONLY)
                .len(workgroups)
                .build()
                .expect("can't create buffer_best_nonces"),
            buffer_best_nonces_diamond: Buffer::<u64>::builder()
                .queue(queue.clone())
                .flags(ocl::core::MEM_WRITE_ONLY)
                .len(workgroups)
                .build()
                .expect("can't create buffer_best_nonces_diamond"),
            buffer_global_hashes: Buffer::<u8>::builder()
                .queue(queue.clone())
                .flags(ocl::core::MEM_READ_WRITE)
                .len(HASH_WIDTH * unitsize as usize * global_work_size as usize)
                .build()
                .expect("can't create buffer_global_hashes"),
            buffer_global_order: Buffer::<u32>::builder()
                .queue(queue.clone())
                .flags(ocl::core::MEM_READ_WRITE)
                .len(unitsize as usize * global_work_size as usize)
                .build()
                .expect("can't create buffer_global_order"),
            buffer_best_hashes: Buffer::<u8>::builder()
                .queue(queue.clone())
                .flags(ocl::core::MEM_WRITE_ONLY)
                .len(HASH_WIDTH * workgroups as usize)
                .build()
                .expect("can't create buffer_best_hashes"),
        });
    }

    opencl_resource_devices
}

pub fn enqueue_kernel_checked(kernel: &Kernel, event: &mut EventList, label: &str) {
    // SAFETY: callers build kernels immediately before enqueue with all arguments bound;
    // buffers and queues are owned by OpenCLResources and outlive the dependent reads.
    unsafe {
        kernel
            .cmd()
            .enew(event)
            .enq()
            .unwrap_or_else(|e| panic!("{}: {}", label, e));
    }
}

fn compile_program_from_source(
    context: &Context,
    device: &Device,
    kernel_path: &Path,
    binary_path: &Path,
    opencldir: &str,
) -> Program {
    let kernel_src = fs::read_to_string(kernel_path).expect("can't find OpenCL kernel file");
    let compile_options = format!("-cl-std=CL2.0 -I {}", opencldir);
    let program = Program::builder()
        .src(&kernel_src)
        .devices(*device)
        .cmplr_opt(compile_options)
        .build(context)
        .expect("OpenCL program compilation failed");

    let program_info_result = program
        .info(ProgramInfo::Binaries)
        .expect("can't read binary data from compiled kernel");
    let binaries = match program_info_result {
        ProgramInfoResult::Binaries(binaries) => binaries,
        _ => panic!("compiled files and binaries don't match"),
    };

    if let Some(binary) = binaries.first() {
        println!("Saving OpenCL program in binary file...");
        let mut binary_file = File::create(binary_path).expect("can't create binary data file");
        binary_file
            .write_all(binary)
            .expect("can't save binary data");
    }

    program
}
