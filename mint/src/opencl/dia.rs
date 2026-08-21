use ocl::{Buffer, EventList, Kernel};

use field::{Address, DiamondName, DiamondNumber, Fixed8, Hash};

use crate::action::diamond::DiamondMint;
use crate::diamond_mining::{
    DiamondMiningResult, HASH_WIDTH, check_diamond_success, diamond_more_power,
};

use super::common::{OpenCLResources, enqueue_kernel_checked};

pub fn do_diamond_group_mining_opencl(
    opencl: &OpenCLResources,
    number: u32,
    prevblockhash: &Hash,
    reward_address: &Address,
    custom_message: &Hash,
    nonce_start: u64,
    nonce_space: u64,
    num_work_groups: u32,
    local_work_size: u32,
    unit_size: u32,
) -> DiamondMiningResult {
    let empty = [0u8; 0];
    let custom_nonce = if number
        > hacash_params::MAINNET_PARAMS
            .mint_rules
            .diamond
            .custom_message_after
    {
        custom_message.as_ref()
    } else {
        &empty
    };
    let mut best = DiamondMiningResult {
        number,
        nonce_start,
        nonce_space,
        nonce: 0,
        diamond_string: [b'W'; 16],
        success: None,
        elapsed_secs: 0.0,
    };

    let global_work_size = num_work_groups.saturating_mul(local_work_size);
    let repeat = x16rs::mine_diamond_hash_repeat(number) as u32;
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
        let mut hash = [0u8; HASH_WIDTH];
        hash.copy_from_slice(&hashes[i * HASH_WIDTH..(i + 1) * HASH_WIDTH]);
        let diamond_string = x16rs::diamond_hash(&hash);
        let nonce_bytes = nonces[i].to_be_bytes();
        let first_hash = x16rs::calculate_hash(
            [
                prevblockhash.as_ref(),
                nonce_bytes.as_slice(),
                reward_address.as_ref(),
                custom_nonce,
            ]
            .concat(),
        );

        if let Some(diamond_name) = check_diamond_success(number, first_hash, hash, diamond_string)
        {
            let mut act =
                DiamondMint::with(DiamondName::from(diamond_name), DiamondNumber::from(number));
            act.d.prev_hash = *prevblockhash;
            act.d.nonce = Fixed8::from(nonce_bytes);
            act.d.address = *reward_address;
            act.d.custom_message = *custom_message;
            best.diamond_string = diamond_string;
            best.nonce = nonces[i];
            best.success = Some(act);
            return best;
        }

        if diamond_more_power(&diamond_string, &best.diamond_string) {
            best.diamond_string = diamond_string;
            best.nonce = nonces[i];
        }
    }

    best
}
