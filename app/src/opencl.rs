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
    buffer_best_nonces: Vec<Buffer::<u32>>,
    buffer_global_hashes: Vec<Buffer::<u8>>,
    buffer_global_order: Vec<Buffer::<u32>>,
    buffer_best_hashes: Vec<Buffer::<u8>>,
}

fn initialize_opencl(cnf: &PoWorkConf) -> OpenCLResources {
    // Binary file location
    let kernel_file = format!(r"{}x16rs_main.cl", cnf.opencldir);
    let kernel_path = Path::new(&kernel_file);

    // Context creation for OpenCL instance
    let platform = Platform::default();

    let name = platform.name().expect("Error");
    let vendor = platform.vendor().expect("Error");
    let version: String = platform.version().expect("Error");
    println!("Platform name: {}", name);
    println!("Manufacturer: {}", vendor);
    println!("Version: {}", version);
    println!("-----------------------------------------");

    let devices = Device::list_all(&platform).expect("Error");
    // Iterate OpenCL devices
    for (idx, device) in devices.iter().enumerate() {
        let name = device.name().expect("Error");
        println!("Device {}: {}", idx, name);
    }
    println!("-----------------------------------------");

    let device = Device::by_idx_wrap(platform, cnf.deviceid.try_into().unwrap()).expect("Can't find OpenCL device");
    let context = Context::builder()
        .platform(platform)
        .devices(device.clone())
        .build()
        .expect("Can't create OpenCL context");
    let device_name = device.name().expect("Can't get device name");

    let binary_file = format!(r"{}{}_{}.bin", cnf.opencldir, device_name, cnf.deviceid);
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
        unsafe {
            Program::with_binary(
                &context,
                &[device.clone()],
                &binaries,
                &CString::new("").unwrap(),
            )
            .expect("Can't create OpenCL program with the binary file")
        }
    } else {
        println!("Compiling...");
        // Compile from source
        compile_program_from_source(&context, &device, &kernel_path, &binary_path, cnf.opencldir.clone())
    };

    // Create new queue
    let queue = Queue::new(&context, device.clone(), Some(QUEUE_OUT_OF_ORDER_EXEC_MODE_ENABLE))
    .expect("Can't create OpenCL event queue");

    let num_work_items = cnf.workgroups * cnf.localsize;
    let global_work_size = num_work_items;

    let mut buffer_best_nonces = Vec::with_capacity(cnf.supervene as usize);
    let mut buffer_global_hashes = Vec::with_capacity(cnf.supervene as usize);
    let mut buffer_global_order = Vec::with_capacity(cnf.supervene as usize);
    let mut buffer_best_hashes = Vec::with_capacity(cnf.supervene as usize);
    for _ in 0..cnf.supervene {
        buffer_best_nonces.push(Buffer::<u32>::builder()
            .queue(queue.clone())
            .flags(ocl::core::MEM_WRITE_ONLY)
            .len(cnf.workgroups)
            .build()
            .expect("Can't create buffer_best_nonces"));

        buffer_global_hashes.push(Buffer::<u8>::builder()
            .queue(queue.clone())
            .flags(ocl::core::MEM_READ_WRITE)
            .len(HASH_WIDTH * cnf.unitsize as usize * global_work_size as usize)
            .build()
            .expect("Can't create buffer_global_hashes"));

        buffer_global_order.push(Buffer::<u32>::builder()
            .queue(queue.clone())
            .flags(ocl::core::MEM_READ_WRITE)
            .len(cnf.unitsize as usize * global_work_size as usize)
            .build()
            .expect("Can't create buffer_global_order"));

        buffer_best_hashes.push(Buffer::<u8>::builder()
            .queue(queue.clone())
            .flags(ocl::core::MEM_WRITE_ONLY)
            .len(HASH_WIDTH * cnf.workgroups as usize )
            .build()
            .expect("Can't create buffer_best_hashes"));
    }

    OpenCLResources {
        program,
        queue,
        buffer_best_nonces,
        buffer_global_hashes,
        buffer_global_order,
        buffer_best_hashes,
    }
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
        .devices(device.clone())
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
