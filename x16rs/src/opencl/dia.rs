use ocl::{Buffer, EventList, Kernel};

use super::HASH_WIDTH;
use super::common::{OpenCLResources, enqueue_kernel_checked};
use crate::{
    calculate_hash, check_diamond_difficulty, check_diamond_hash_result, diamond_hash,
    diamond_more_power, mine_diamond_hash_repeat,
};

/// Hash-level result of one diamond kernel run: the strongest candidate
/// ("more power" comparison) among all work groups, plus `name` when it meets
/// the target difficulty. Building the mint action from this stays with the
/// caller (policy/domain layer).
pub struct DiamondKernelResult {
    pub nonce: u64,
    pub first_hash: [u8; HASH_WIDTH],
    pub result_hash: [u8; HASH_WIDTH],
    pub diamond_string: [u8; 16],
    pub name: Option<[u8; 6]>,
}

/// Launch one diamond mining pass on the GPU. `custom_nonce` is the already
/// resolved custom message slice (the caller applies the params rule for when
/// custom messages are allowed).
#[allow(clippy::too_many_arguments)]
pub fn do_diamond_group_mining_opencl(
    opencl: &OpenCLResources,
    number: u32,
    prevblockhash: &[u8; HASH_WIDTH],
    reward_address: &[u8; 21],
    custom_nonce: &[u8],
    nonce_start: u64,
    num_work_groups: u32,
    local_work_size: u32,
    unit_size: u32,
) -> DiamondKernelResult {
    let mut best = DiamondKernelResult {
        nonce: 0,
        first_hash: [0u8; HASH_WIDTH],
        result_hash: [0u8; HASH_WIDTH],
        diamond_string: [b'W'; 16],
        name: None,
    };

    let global_work_size = num_work_groups.saturating_mul(local_work_size);
    let repeat = mine_diamond_hash_repeat(number) as u32;
    let stuff = [
        prevblockhash.as_ref(),
        [0u8; 8].as_slice(),
        reward_address.as_ref(),
        custom_nonce,
    ]
    .concat();

    let buffer_block_intro = Buffer::<u8>::builder()
        .queue(opencl.queue.clone())
        .flags(ocl::core::MEM_READ_ONLY)
        .len(stuff.len())
        .copy_host_slice(&stuff)
        .build()
        .expect("unable to create diamond OpenCL input buffer");

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
        .expect("unable to build diamond OpenCL kernel");

    let mut kernel_event = EventList::new();
    enqueue_kernel_checked(
        &kernel,
        &mut kernel_event,
        "unable to queue diamond OpenCL kernel",
    );

    let mut hashes = vec![0u8; opencl.buffer_best_hashes.len()];
    opencl
        .buffer_best_hashes
        .read(&mut hashes)
        .ewait(&kernel_event)
        .enq()
        .expect("can't read diamond best hashes");

    let mut nonces = vec![0u64; opencl.buffer_best_nonces_diamond.len()];
    opencl
        .buffer_best_nonces_diamond
        .read(&mut nonces)
        .ewait(&kernel_event)
        .enq()
        .expect("can't read diamond best nonces");

    for i in 0..num_work_groups as usize {
        let mut result_hash = [0u8; HASH_WIDTH];
        result_hash.copy_from_slice(&hashes[i * HASH_WIDTH..(i + 1) * HASH_WIDTH]);
        let diamond_string = diamond_hash(&result_hash);
        let nonce = nonces[i];
        let nonce_bytes = nonce.to_be_bytes();
        let first_hash = calculate_hash(
            [
                prevblockhash.as_ref(),
                nonce_bytes.as_slice(),
                reward_address.as_ref(),
                custom_nonce,
            ]
            .concat(),
        );

        if let Some(diamond_name) =
            check_diamond_hash_result(diamond_string)
                .filter(|_| check_diamond_difficulty(number, &first_hash, &result_hash))
        {
            return DiamondKernelResult {
                nonce,
                first_hash,
                result_hash,
                diamond_string,
                name: Some(diamond_name),
            };
        }

        if diamond_more_power(&diamond_string, &best.diamond_string) {
            best.nonce = nonce;
            best.first_hash = first_hash;
            best.result_hash = result_hash;
            best.diamond_string = diamond_string;
        }
    }

    best
}
