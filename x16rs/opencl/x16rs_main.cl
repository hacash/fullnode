#include "util.cl"
#include "x16rs.cl"
#include "sha3_256.cl"

inline int diff_big_hash(__generic hash_32 *src, __generic hash_32 *tar)
{
    #pragma unroll 32
    for (int i = 0; i < 32; i++) {
        if (src->h1[i] > tar->h1[i]) {
            return 1;
        } else if (src->h1[i] < tar->h1[i]) {
            return 0;
        }
    }
    return 0;
}

__attribute__((work_group_size_hint(256, 1, 1)))
__kernel void x16rs_main(
    __constant block_t* input_stuff_89,
    const unsigned int nonce_start,
    const unsigned int x16rs_repeat,
    const unsigned int item_loop,
    const unsigned int unit_size,
    __global hash_32* global_hashes,
    __global unsigned int* global_nonces,
    __global unsigned int* global_order,
    __global hash_32* best_hashes,
    __global unsigned int* best_nonces
) {
    const unsigned int local_id = get_local_id(0);
    const unsigned int local_size = get_local_size(0);
    const unsigned int group_id = get_group_id(0);
    const unsigned int index = local_id * unit_size;
    hash_32* local_hashes = global_hashes + (group_id * local_size * unit_size);
    unsigned int* local_nonces = global_nonces + (group_id * local_size * unit_size);
    unsigned int* local_order = global_order + (group_id * local_size * unit_size);
    __local unsigned int histogram[16];
    __local unsigned int starting_index[16];
    __local unsigned int offset[16];

    // Setup x16 local shared vars
    __constant sph_u64 H_blake[8] = {
        SPH_C64(0x6A09E667F3BCC908), SPH_C64(0xBB67AE8584CAA73B),
        SPH_C64(0x3C6EF372FE94F82B), SPH_C64(0xA54FF53A5F1D36F1),
        SPH_C64(0x510E527FADE682D1), SPH_C64(0x9B05688C2B3E6C1F),
        SPH_C64(0x1F83D9ABFB41BD6B), SPH_C64(0x5BE0CD19137E2179)
    };
    __local ulong T0[256], T1[256], T2[256], T3[256];
    __local sph_u32 AES0[256], AES1[256], AES2[256], AES3[256];
    __local sph_u64 LT0[256], LT1[256], LT2[256], LT3[256], LT4[256], LT5[256], LT6[256], LT7[256];
    __local sph_u32 mixtab0[256], mixtab1[256], mixtab2[256], mixtab3[256];
    for (unsigned i = local_id; i < 256; i += local_size) {
        T0[i] = T0_G[i];
        T1[i] = rotate(T0[i], 8UL);
        T2[i] = rotate(T0[i], 16UL);
        T3[i] = rotate(T0[i], 24UL);
        
        AES0[i] = AES0_C[i];
        AES1[i] = rotate(AES0[i], 8U);
        AES2[i] = rotate(AES0[i], 16U);
        AES3[i] = rotate(AES0[i], 24U);

        LT0[i] = plain_T0[i];
        LT1[i] = plain_T1[i];
        LT2[i] = plain_T2[i];
        LT3[i] = plain_T3[i];
        LT4[i] = plain_T4[i];
        LT5[i] = plain_T5[i];
        LT6[i] = plain_T6[i];
        LT7[i] = plain_T7[i];

        mixtab0[i] = mixtab0_c[i];
        mixtab1[i] = mixtab1_c[i];
        mixtab2[i] = mixtab2_c[i];
        mixtab3[i] = mixtab3_c[i];
    }
    barrier(CLK_LOCAL_MEM_FENCE);

    block_t base_stuff = input_stuff_89[0];
    
    hash_32 best_hash = { 0 };
    unsigned int best_nonce = 0;
    const unsigned int global_offset = nonce_start + (get_global_id(0) * unit_size * item_loop);
    for(unsigned char loop = 0; loop < item_loop; loop++) {
        for (unsigned int i = 0; i < unit_size; i++) {
            // Insert Nonce
            local_nonces[index + i] = global_offset + (unit_size * loop) + i;
            write_nonce_to_bytes(79, base_stuff.h1, local_nonces[index + i]);
            // Hash Block
            sha3_256_hash(base_stuff.h8, local_hashes[index + i].h8);
        }

        barrier(CLK_LOCAL_MEM_FENCE);

        for (unsigned char r = 0; r < x16rs_repeat; r++) {

            // Reset sorting indices
            if (local_id < 16) {
                histogram[local_id] = 0;
                offset[local_id] = 0;
            }
            barrier(CLK_LOCAL_MEM_FENCE);

            // Sumarize by algo
            for (unsigned int h = 0; h < unit_size; h++) {
                unsigned char mod = local_hashes[index + h].h4[7] % 16;
                atomic_inc(&histogram[mod]);
            }
            barrier(CLK_LOCAL_MEM_FENCE);

            // Calculate start index
            if (local_id == 0) {
                starting_index[0] = 0;
                for (unsigned char i = 1; i < 16; i++) {
                    starting_index[i] = starting_index[i - 1] + histogram[i - 1];
                }
            }
            barrier(CLK_LOCAL_MEM_FENCE);

            // Save each hash position
            for (unsigned int h = 0; h < unit_size; h++) {
                unsigned int mod = local_hashes[index + h].h4[7] % 16;
                unsigned int pos = starting_index[mod] + atomic_inc(&offset[mod]);
                local_order[pos] = index + h;
            }
            barrier(CLK_LOCAL_MEM_FENCE);

            for(unsigned int h = 0; h < unit_size; h++) {
                const unsigned int hash_pos = local_order[(local_size * h) + local_id];
                switch(local_hashes[hash_pos].h4[7] % 16) {
                    case 0:
                        hash_x16rs_func_0(&local_hashes[hash_pos], H_blake);
                        break;
                    case 1:
                        hash_x16rs_func_1(&local_hashes[hash_pos]);
                        break;
                    case 2:
                        hash_x16rs_func_2(&local_hashes[hash_pos], T0, T1, T2, T3);
                        break;
                    case 3:
                        hash_x16rs_func_3(&local_hashes[hash_pos]);
                        break;
                    case 4:
                        hash_x16rs_func_4(&local_hashes[hash_pos]);
                        break;
                    case 5:
                        hash_x16rs_func_5(&local_hashes[hash_pos]);
                        break;
                    case 6:
                        hash_x16rs_func_6(&local_hashes[hash_pos]);
                        break;
                    case 7:
                        hash_x16rs_func_7(&local_hashes[hash_pos]);
                        break;
                    case 8:
                        hash_x16rs_func_8(&local_hashes[hash_pos], AES0, AES1, AES2, AES3);
                        break;
                    case 9:
                        hash_x16rs_func_9(&local_hashes[hash_pos]);
                        break;
                    case 10:
                        hash_x16rs_func_10(&local_hashes[hash_pos], AES0, AES1, AES2, AES3);
                        break;
                    case 11:
                        hash_x16rs_func_11(&local_hashes[hash_pos]);
                        break;
                    case 12:
                        hash_x16rs_func_12(&local_hashes[hash_pos], mixtab0, mixtab1, mixtab2, mixtab3);
                        break;
                    case 13:
                        hash_x16rs_func_13(&local_hashes[hash_pos]);
                        break;
                    case 14:
                        hash_x16rs_func_14(&local_hashes[hash_pos], LT0, LT1, LT2, LT3, LT4, LT5, LT6, LT7);
                        break;
                    case 15:
                        hash_x16rs_func_15(&local_hashes[hash_pos]);
                        break;
                }
                barrier(CLK_LOCAL_MEM_FENCE);
            }
        }

        for (unsigned int i = 0; i < unit_size; i++) {
            unsigned int idx = index + i;
            if ((loop == 0 && i == 0) || diff_big_hash(&best_hash, &local_hashes[idx]) == 1) {
                best_hash = local_hashes[idx];
                best_nonce = local_nonces[idx];
            }
        }
        barrier(CLK_LOCAL_MEM_FENCE);
    }

    // Store the best hash at the start of the block
    local_hashes[index] = best_hash;
    local_nonces[index] = best_nonce;
    
    barrier(CLK_LOCAL_MEM_FENCE);

    // Now perform the reduction across threads
    for (unsigned int smax = local_size >> 1; smax > 0; smax >>= 1) {
        if (local_id < smax) {
            unsigned int idx_current = index;
            unsigned int idx_pair = (local_id + smax) * unit_size;
            if (diff_big_hash(&local_hashes[idx_current], &local_hashes[idx_pair]) == 1) {
                local_hashes[idx_current] = local_hashes[idx_pair];
                local_nonces[idx_current] = local_nonces[idx_pair];
            }
        }
        barrier(CLK_LOCAL_MEM_FENCE);
    }

    if(local_id == 0) {
        best_hashes[group_id] = local_hashes[0];
        best_nonces[group_id] = local_nonces[0];
    }
}