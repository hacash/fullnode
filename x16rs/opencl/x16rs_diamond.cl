#include "util.cl"
#include "x16rs.cl"
#include "sha3_256.cl"

// Diamond name calculation constants
#define DMD_M 16
#define H32S 32
#define DIAMOND_HASH_BASE_CHAR_NUM 17
__constant uchar DIAMOND_HASH_BASE_CHARS[DIAMOND_HASH_BASE_CHAR_NUM] = "0WTYUIAHXVMEKBSZN";

inline int diff_big_hash(const hash_32 *src, const hash_32 *tar)
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

inline void diamond_hash(const uchar *bshash, uchar *reshx)
{
    uint mgcidx = 13u;
    for (uint i = 0u; i < (uint)DMD_M; i++) {
        uint a = (uint)bshash[i * 2u];
        uint b = (uint)bshash[i * 2u + 1u];
        uint num = mgcidx * a * b;
        mgcidx = num % (uint)DIAMOND_HASH_BASE_CHAR_NUM;
        reshx[i] = DIAMOND_HASH_BASE_CHARS[mgcidx];
        if (mgcidx == 0u) mgcidx = 13u;
    }
}

inline bool diamond_more_power(const uchar *dst,
                                 const uchar *src)
{
     const uchar o = (uchar)'0';
    for (uint i = 0u; i < 16u; i++) {
        uchar l = dst[i];
        uchar r = src[i];
        if (l == o && r != o) return 1;
        if (l != o && r == o) return 0;
        if (l != o && r != o) return 0;
    }
    return 0;
}

__attribute__((work_group_size_hint(256, 1, 1)))
__kernel void x16rs_diamond(
    __constant const block_diamond_t* input_stuff,
    const ulong nonce_start,
    const unsigned int x16rs_repeat,
    const unsigned int unit_size,
    __global hash_32* global_hashes,
    __global unsigned int* global_order,
    __global hash_32* best_hashes,
    __global ulong* best_nonces
) {
    const unsigned int local_id = get_local_id(0);
    const unsigned int local_size = get_local_size(0);
    const unsigned int group_id = get_group_id(0);
    const unsigned int index = local_id * unit_size;
    hash_32* local_hashes = global_hashes + (group_id * local_size * unit_size);
    __local ulong local_nonces[256];
    __local diamond_t local_names[256];
    __global unsigned int* local_order = global_order + (group_id * local_size * unit_size);
    __local unsigned int ALIGN histogram[16];
    __local unsigned int ALIGN starting_index[16];
    __local unsigned int ALIGN offset[16];

    // Setup x16 local shared vars
    X16RS_INIT_SHARED_TABLES(local_id, local_size);

    block_diamond_t base_stuff = input_stuff[0];
    
    const ulong global_offset = nonce_start + (get_global_id(0) * unit_size);
    for (unsigned int i = 0; i < unit_size; i++) {
        volatile const ulong nonce = global_offset + i;
        if(false) {
            // Insert Nonce
            write_nonce_to_bytes(79, base_stuff.h1, nonce);
        } else {
            write_nonce_u64_to_bytes(32, base_stuff.h1, nonce);
        }
        // Hash Block
        sha3_256_hash_diamond(base_stuff.h8, local_hashes[index + i].h8);
    }          
    barrier(CLK_LOCAL_MEM_FENCE);

    X16RS_RUN_REPEAT_LOOP(
        local_id, local_size, unit_size, x16rs_repeat,
        local_hashes, index, local_order,
        histogram, starting_index, offset,
        H_blake,
        T0, T1, T2, T3,
        AES0, AES1, AES2, AES3,
        LT0, LT1, LT2, LT3, LT4, LT5, LT6, LT7,
        mixtab0, mixtab1, mixtab2, mixtab3
    );
    
    unsigned int best_hash = 0;
    diamond_t best_name;
    for (unsigned int i = 1; i < unit_size; i++) {
        // Get diamond name
        diamond_t now_name;
        diamond_hash(local_hashes[index + i].h1, now_name.h1);
        if (diamond_more_power(now_name.h1, best_name.h1) == 1) {
            best_hash = index + i;
            best_name = now_name;
        }
    }
    barrier(CLK_LOCAL_MEM_FENCE);

    local_hashes[index] = local_hashes[best_hash];
    local_nonces[local_id] = global_offset + best_hash - index;
    
    barrier(CLK_LOCAL_MEM_FENCE);

    // Now perform the reduction across threads
    for (unsigned int smax = local_size >> 1; smax > 0; smax >>= 1) {
        if (local_id < smax) {
            unsigned int idx_current = index;
            unsigned int idx_pair = (local_id + smax) * unit_size;
            diamond_t current_name;
            diamond_t pair_name;
            diamond_hash(local_hashes[idx_current].h1, current_name.h1);
            diamond_hash(local_hashes[idx_pair].h1, pair_name.h1);
            if (diamond_more_power(pair_name.h1, current_name.h1) == 1) {
                local_hashes[idx_current] = local_hashes[idx_pair];
                local_nonces[local_id] = local_nonces[local_id + smax];
            }
        }
        barrier(CLK_LOCAL_MEM_FENCE);
    }

    if(local_id == 0) {
        best_nonces[group_id] = local_nonces[0];
    }
    if(local_id < 32) {
        best_hashes[group_id].h1[local_id] = local_hashes[0].h1[local_id];
    }
}
