use std::ffi::CString;
use std::path::Path;
use std::fs::{self, File};
use std::io::{Read, Write};
use ocl::core::QUEUE_OUT_OF_ORDER_EXEC_MODE_ENABLE;
use ocl::enums::{ProgramInfoResult, ProgramInfo};
use ocl::{Buffer, Context, Device, EventList, Kernel, Platform, Program, Queue};

struct OpenCLResources {
    program: Program,
    queue: Queue,
    buffer_best_nonces: Buffer::<u32>,
    buffer_global_hashes: Buffer::<u8>,
    buffer_global_order: Buffer::<u32>,
    buffer_best_hashes: Buffer::<u8>,
}

fn initialize_opencl(cnf: &PoWorkConf) -> Vec<OpenCLResources> {
    // Binary file location
    let kernel_file = format!(r"{}x16rs_main.cl", cnf.opencldir);
    let kernel_path = Path::new(&kernel_file);

    // Context creation for OpenCL instance
    let platforms = Platform::list();
    let platform = platforms
        .get(cnf.platformid as usize)
        .expect("The specified platform id is invalid")
        .clone();

    let name = platform.name().expect("Error");
    let vendor = platform.vendor().expect("Error");
    let version: String = platform.version().expect("Error");
    println!("Platform name: {}", name);
    println!("Manufacturer: {}", vendor);
    println!("Version: {}", version);

    let mut cnf_devices: Vec<u32> = cnf.deviceids.split(',')
        .filter(|s| !s.trim().is_empty())
        .filter_map(|s| s.trim().parse::<u32>().ok())
        .collect();

    // Set all devices when empty
    if cnf_devices.is_empty() {
        let platform_devices = Device::list_all(&platform).expect("Error getting device list");
        // Iterate all OpenCL devices
        for (idx, _) in platform_devices.iter().enumerate() {
            cnf_devices.push(idx as u32);
        }
    }

    // Create Device vector
    let mut devices: Vec<Device> = [].to_vec();
    for (_, &device_id) in cnf_devices.iter().enumerate() {
        let device = Device::by_idx_wrap(platform, device_id.try_into().unwrap()).expect("Can't find OpenCL device");
        devices.push(device);
    }

    let num_work_items = cnf.workgroups * cnf.localsize;
    let global_work_size = num_work_items;

    let mut opencl_resource_devices = Vec::with_capacity(devices.len() as usize);
    for (idx, &device) in devices.iter().enumerate() {
        
        println!("-----------------------------------------");
        let name = device.name().expect("Error");
        println!("Device {}: {}", cnf_devices[idx], name);
        println!("-----------------------------------------");
        
        // Create context
        let context = Context::builder()
            .platform(platform)
            .devices(device)
            .build()
            .expect("Can't create OpenCL context");

        if !Path::new(&cnf.opencldir).is_dir() {
            panic!("OpenCL dir not found: {}", cnf.opencldir);
        }

        let device_name = device.name().expect("Can't get device name");
        let binary_file = format!(r"{}{}_{}.bin", cnf.opencldir, device_name, cnf_devices[idx]);
        let binary_path = Path::new(&binary_file);

        // Check if kernel was changed since last time (and need recompile)
        let need_recompile = if binary_path.exists() {
            let binary_modified = fs::metadata(&binary_path)
                .and_then(|meta| meta.modified())
                .expect("Can't find binary file last edit time");
            let kernel_modified = fs::metadata(&kernel_path)
                .and_then(|meta| meta.modified())
                .expect("Can't find kernel file last edit time");
            kernel_modified > binary_modified
        } else {
            true
        };

        let program = if !need_recompile {
            // Read program from binary file
            let mut binary_file = File::open(&binary_path).expect("No se pudo abrir el archivo binario");
            let mut binary_data = Vec::new();
            binary_file
                .read_to_end(&mut binary_data)
                .expect("Can't read binary file");
            println!("Loading OpenCL from the binary...");
            let binaries = [&binary_data[..]];
            Program::with_binary(
                &context,
                &[device],
                &binaries,
                &CString::new("").unwrap(),
            )
            .expect("Can't create OpenCL program with the binary file")
        } else {
            println!("Compiling...");
            // Compile from source
            compile_program_from_source(&context, &device, &kernel_path, &binary_path, cnf.opencldir.clone())
        };
        
        // Create new queue
        let queue = Queue::new(&context, device.clone(), Some(QUEUE_OUT_OF_ORDER_EXEC_MODE_ENABLE))
        .expect("Can't create OpenCL event queue");

        opencl_resource_devices.push(OpenCLResources {
            program: program.clone(),
            queue: queue.clone(),
            buffer_best_nonces: Buffer::<u32>::builder()
                .queue(queue.clone())
                .flags(ocl::core::MEM_WRITE_ONLY)
                .len(cnf.workgroups)
                .build()
                .expect("Can't create buffer_best_nonces"),
            buffer_global_hashes: Buffer::<u8>::builder()
                .queue(queue.clone())
                .flags(ocl::core::MEM_READ_WRITE)
                .len(HASH_WIDTH * cnf.unitsize as usize * global_work_size as usize)
                .build()
                .expect("Can't create buffer_global_hashes"),
            buffer_global_order: Buffer::<u32>::builder()
                .queue(queue.clone())
                .flags(ocl::core::MEM_READ_WRITE)
                .len(cnf.unitsize as usize * global_work_size as usize)
                .build()
                .expect("Can't create buffer_global_order"),
            buffer_best_hashes: Buffer::<u8>::builder()
                .queue(queue.clone())
                .flags(ocl::core::MEM_WRITE_ONLY)
                .len(HASH_WIDTH * cnf.workgroups as usize )
                .build()
                .expect("Can't create buffer_best_hashes")
        });
    }

    opencl_resource_devices
}

fn compile_program_from_source(
    context: &Context,
    device: &Device,
    kernel_path: &Path,
    binary_path: &Path,
    opencldir: String,
) -> Program {
    // Create program from source files
    let kernel_src = fs::read_to_string(kernel_path)
        .expect("Can't find kernel file");

    // Compile
    let compile_options = format!(r"-cl-std=CL2.0 -I {}", opencldir);
    let program_build = Program::builder()
        .src(&kernel_src)
        .devices(device)
        .cmplr_opt(compile_options)
        .build(context);

    let program: Program = match program_build {
        Ok(prog) => {
            prog
        }
        Err(e) => {
            eprintln!("OpenCL program compilation error: {}", e);
            panic!("OpenCL program compilation failed");
        }
    };

    // Get the binary result and save in file
    let program_info_result = program
        .info(ProgramInfo::Binaries)
        .expect("Can't read binary data from compiled kernel");

    // Extract Vec<Vec<u8>> from ProgramInfoResult enum
    let binaries = match program_info_result {
        ProgramInfoResult::Binaries(binaries) => binaries,
        _ => {
            panic!("Compiled files and binaries doesn't match");
        }
    };

    if let Some(binary) = binaries.get(0) {
        println!("Saving OpenCL program in binary file...");
        let mut binary_file = File::create(binary_path)
            .expect("Can't create binary data file");
        binary_file
            .write_all(binary)
            .expect("Can't save binary data");
    } else {
        println!("Can't find binaries from program");
    }

    program
}

fn do_group_block_mining_opencl(
    opencl: &OpenCLResources,
    height: u64,
    block_intro: Vec<u8>,
    nonce_start: u32,
    num_work_groups: u32,
    local_work_size: u32,
    unit_size: u32,
) -> (u32, [u8; 32]) {
    let mut most_nonce = 0u32;
    let mut most_hash = [255u8; 32];
    let global_work_size = num_work_groups * local_work_size;
    let repeat = x16rs::block_hash_repeat(height) as u32;

    let buffer_block_intro = Buffer::<u8>::builder()
        .queue(opencl.queue.clone())
        .flags(ocl::core::MEM_READ_ONLY)
        .len(block_intro.len())
        .copy_host_slice(&block_intro)
        .build()
        .expect("Unable to create buffer_block_intro");

    let kernel = Kernel::builder()
        .program(&opencl.program)
        .name("x16rs_main")
        .queue(opencl.queue.clone())
        .global_work_size(global_work_size)
        .local_work_size(local_work_size)
        .arg(&buffer_block_intro)
        .arg(nonce_start)
        .arg(repeat)
        .arg(unit_size)
        .arg(&opencl.buffer_global_hashes)
        .arg(&opencl.buffer_global_order)
        .arg(&opencl.buffer_best_hashes)
        .arg(&opencl.buffer_best_nonces)
        .build()
        .unwrap();

    let mut kernel_event = EventList::new();
    unsafe {
        kernel.cmd().enew(&mut kernel_event).enq().expect("Unable to queue OpenCL kernel");
    }

    let mut hashes = vec![0u8; opencl.buffer_best_hashes.len()];
    opencl.buffer_best_hashes
        .read(&mut hashes)
        .ewait(&kernel_event)
        .enq()
        .expect("Can't read buffer_best_hashes");

    let mut nonces = vec![0u32; opencl.buffer_best_nonces.len()];
    opencl.buffer_best_nonces
        .read(&mut nonces)
        .ewait(&kernel_event)
        .enq()
        .expect("Can't read buffer_best_nonces");

    for i in 0..num_work_groups as usize {
        let hash_bytes = &hashes[i * 32..(i * 32) + 32];
        if hash_more_power(hash_bytes, &most_hash) {
            most_hash.copy_from_slice(hash_bytes);
            most_nonce = nonces[i];
        }
    }
    
    (most_nonce, most_hash)
}
