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
