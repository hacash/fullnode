use x16rs::diamond_hash;

fn do_diamond_group_mining_opencl(
    opencl: &OpenCLResources,
    number: u32,
    prevblockhash: &Hash, 
    rwdaddr: &Address,
    custom_message: &Hash,
    nonce_start: u64,
    nonce_space: u64,
    num_work_groups: u32,
    local_work_size: u32,
    unit_size: u32,
) -> DiamondMiningResult {
    let empthbytes = [0u8; 0];
    let prevhash: &[u8; HASH_WIDTH] = prevblockhash;
    let address: &[u8; 21] = rwdaddr;
    let custom_nonce: &[u8] = match number > DIAMOND_ABOVE_NUMBER_OF_CREATE_BY_CUSTOM_MESSAGE {
        true => custom_message.as_bytes(),
        false => &empthbytes,
    };
    let mut most = DiamondMiningResult {
        number,
        nonce_start,
        nonce_space,
        u64_nonce: 0,
        msg_nonce: custom_nonce.to_vec(),
        dia_str: [b'W'; 16],
        is_success: None,
        use_secs: 0.0,
    };
    let global_work_size = num_work_groups * local_work_size;
    let repeat = x16rs::mine_diamond_hash_repeat(number) as u32;
    let stuff = [
        prevhash.to_vec(),
        [0u8; 8].to_vec(),
        address.to_vec(),
        custom_nonce.as_ref().to_vec(),
    ].concat();

    let buffer_block_intro = Buffer::<u8>::builder()
        .queue(opencl.queue.clone())
        .flags(ocl::core::MEM_READ_ONLY)
        .len(stuff.len())
        .copy_host_slice(&stuff)
        .build()
        .expect("Unable to create buffer_block_intro");

    let kernel = Kernel::builder()
        .program(&opencl.program)
        .name("x16rs_diamond")
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
        .arg(&opencl.buffer_best_nonces_diamond)
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

    let mut nonces = vec![0u64; opencl.buffer_best_nonces_diamond.len()];
    opencl.buffer_best_nonces_diamond
        .read(&mut nonces)
        .ewait(&kernel_event)
        .enq()
        .expect("Can't read buffer_best_nonces_diamond");

    for i in 0..num_work_groups as usize {
        let hash_bytes = &hashes[i * 32..(i * 32) + 32].try_into().unwrap();
        let dia_str = diamond_hash(&hash_bytes);
        let nonce_bytes = nonces[i].to_be_bytes();
        let stuff = [
            prevblockhash.as_slice(),
            nonce_bytes.as_slice(),
            address.as_slice(),
            custom_message.as_ref(),
        ].concat();
        let ssshash: [u8; 32] = calculate_hash(stuff);
        
        if let Some(dia_name) = check_diamer_success(number, ssshash, *hash_bytes, dia_str) {
            let name = DiamondName::from(dia_name);
            let number = DiamondNumber::from(number);
            let mut diamint = DiamondMint::with(name, number);
            diamint.d.prev_hash = prevblockhash.clone();
            diamint.d.nonce = Fixed8::from(nonces[i].to_be_bytes());
            diamint.d.address = rwdaddr.clone();
            diamint.d.custom_message = custom_message.clone();
            most.dia_str = dia_str;
            most.u64_nonce = nonces[i];
            most.is_success = Some(diamint);
            return most;
        } else if diamond_more_power(&dia_str, &most.dia_str) {
            most.dia_str = dia_str;
            most.u64_nonce = nonces[i];
        }
    }
    most
}
