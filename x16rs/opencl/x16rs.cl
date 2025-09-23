#ifndef X16RX_CL
#define X16RX_CL

#if __ENDIAN_LITTLE__
  #define SPH_LITTLE_ENDIAN 1
#else
  #define SPH_BIG_ENDIAN 1
#endif

typedef unsigned int sph_u32;
typedef int sph_s32;
#ifndef __OPENCL_VERSION__
  typedef unsigned long long sph_u64;
  typedef long long sph_s64;
#else
  typedef unsigned long sph_u64;
  typedef long sph_s64;
#endif

#define SPH_64 1

#define SPH_C32(x)    ((sph_u32)(x ## U))
#define SPH_T32(x) (as_uint(x))
#define SPH_ROTL32(x, n) rotate(as_uint(x), as_uint(n))
#define SPH_ROTR32(x, n)   SPH_ROTL32(x, (32 - (n)))

#define SPH_C64(x)    ((sph_u64)(x ## UL))
#define SPH_T64(x) (as_ulong(x))
#define SPH_ROTL64(x, n) rotate(as_ulong(x), (n) & 0xFFFFFFFFFFFFFFFFUL)
#define SPH_ROTR64(x, n)   SPH_ROTL64(x, (64 - (n)))

#define SPH_JH_64 1
#define SPH_KECCAK_64 1
#define SPH_KECCAK_UNROLL 8
#define SPH_KECCAK_NOCOPY 0
#define SPH_LUFFA_PARALLEL 1
#define SPH_ECHO_64 1
#define SPH_SMALL_FOOTPRINT_HAMSI 0
#define SPH_HAMSI_SHORT 1
#define SPH_HAMSI_EXPAND_BIG 1
#define NO_AMD_OPS 1
#define SPH_COMPACT_BLAKE_64 0
#define SPH_SIMD_NOCOPY 0
#define SPH_SMALL_FOOTPRINT_JH 1

#include "blake.cl"
#include "bmw.cl"
#include "groestl.cl"
#include "skein.cl"
#include "jh.cl"
#include "keccak.cl"
#include "luffa.cl"
#include "cubehash.cl"
#include "shavite.cl"
#include "simd.cl"
#include "echo.cl"
#include "hamsi_help.cl"
#include "hamsi.cl"
#include "fugue.cl"
#include "shabal.cl"
#include "whirlpool.cl"
#include "sha2_512.cl"

#define SWAP4(x) as_uint(as_uchar4(x).wzyx)
#define SWAP8(x) as_ulong(as_uchar8(x).s76543210)

#if SPH_BIG_ENDIAN
  #define DEC64E(x) (x)
  #define DEC32E(x) (x)
  #define DEC64BE(x) (*(const __global sph_u64 *) (x))
  #define DEC32LE(x) SWAP4(*(const __global sph_u32 *) (x))
#else
  #define DEC64E(x) SWAP8(x)
  #define DEC32E(x) SWAP4(x)
  #define DEC64BE(x) SWAP8(*(const __global sph_u64 *) (x))
  #define DEC32LE(x) (*(const __global sph_u32 *) (x))
#endif

#define ENC64E DEC64E
#define ENC32E DEC32E

#define SHL(x, n) ((x) << (n))
#define SHR(x, n) ((x) >> (n))

#define CONST_EXP2  q[i+0] + SPH_ROTL64(q[i+1], 5)  + q[i+2] + SPH_ROTL64(q[i+3], 11) + \
                    q[i+4] + SPH_ROTL64(q[i+5], 27) + q[i+6] + SPH_ROTL64(q[i+7], 32) + \
                    q[i+8] + SPH_ROTL64(q[i+9], 37) + q[i+10] + SPH_ROTL64(q[i+11], 43) + \
                    q[i+12] + SPH_ROTL64(q[i+13], 53) + (SHR(q[i+14],1) ^ q[i+14]) + (SHR(q[i+15],2) ^ q[i+15])

typedef union ALIGN {
  unsigned char h1[64];
  uint h4[16];
  ulong h8[8];
} hash_t;

typedef union ALIGN {
  unsigned char h1[32];
  uint h4[8];
  ulong h8[4];
} hash_32;

// blake
void hash_x16rs_func_0(hash_32* hash, __constant sph_u64* H_blake)
{
    sph_u64 H0 = H_blake[0], H1 = H_blake[1], H2 = H_blake[2], H3 = H_blake[3];
    sph_u64 H4 = H_blake[4], H5 = H_blake[5], H6 = H_blake[6], H7 = H_blake[7];
    sph_u64 S0 = 0, S1 = 0, S2 = 0, S3 = 0;
    sph_u64 T0 = SPH_C64(0x0000000000000100), T1 = 0;

    sph_u64 M0, M1, M2, M3, M4, M5, M6, M7;
    sph_u64 M8, M9, MA, MB, MC, MD, ME, MF;
    sph_u64 V0, V1, V2, V3, V4, V5, V6, V7;
    sph_u64 V8, V9, VA, VB, VC, VD, VE, VF;

    M0 = SWAP8(hash->h8[0]);
    M1 = SWAP8(hash->h8[1]);
    M2 = SWAP8(hash->h8[2]);
    M3 = SWAP8(hash->h8[3]);
    M4 = SPH_C64(0x8000000000000000);
    M5 = 0;
    M6 = 0;
    M7 = 0;
    M8 = 0;
    M9 = 0;
    MA = 0;
    MB = 0;
    MC = 0;
    MD = SPH_C64(0x0000000000000001);
    ME = 0;
    MF = SPH_C64(0x0000000000000100);

    COMPRESS64;

    hash->h8[0] = SWAP8(H0);
    hash->h8[1] = SWAP8(H1);
    hash->h8[2] = SWAP8(H2);
    hash->h8[3] = SWAP8(H3);
}

// bmw
void hash_x16rs_func_1(hash_32* hash)
{
    sph_u64 ALIGN64 BMW_H[16] = { BMW_IV512[0], BMW_IV512[1], BMW_IV512[2], BMW_IV512[3], BMW_IV512[4], BMW_IV512[5], BMW_IV512[6], BMW_IV512[7], BMW_IV512[8], BMW_IV512[9], BMW_IV512[10], BMW_IV512[11], BMW_IV512[12], BMW_IV512[13], BMW_IV512[14], BMW_IV512[15] };

    sph_u64 ALIGN64 mv[16] = { 0 };
    sph_u64 ALIGN64 q[32] = { 0 };

    mv[0] = hash->h8[0];
    mv[1] = hash->h8[1];
    mv[2] = hash->h8[2];
    mv[3] = hash->h8[3];
    mv[4] = SPH_C64(0x0000000000000080);
    mv[15] = SPH_C64(0x000000000000100);

    sph_u64 tmp = BMW_H[5] - BMW_H[7] + BMW_H[10] + BMW_H[13] + BMW_H[14];
    q[0] = (SHR(tmp, 1) ^ SHL(tmp, 3) ^ SPH_ROTL64(tmp, 4) ^ SPH_ROTL64(tmp, 37)) + BMW_H[1];
    tmp = BMW_H[6] - BMW_H[8] + BMW_H[11] + BMW_H[14] - (mv[15] ^ BMW_H[15]);
    q[1] = (SHR(tmp, 1) ^ SHL(tmp, 2) ^ SPH_ROTL64(tmp, 13) ^ SPH_ROTL64(tmp, 43)) + BMW_H[2];
    tmp = (mv[0] ^ BMW_H[0]) + BMW_H[7] + BMW_H[9] - BMW_H[12] + (mv[15] ^ BMW_H[15]);
    q[2] = (SHR(tmp, 2) ^ SHL(tmp, 1) ^ SPH_ROTL64(tmp, 19) ^ SPH_ROTL64(tmp, 53)) + BMW_H[3];
    tmp = (mv[0] ^ BMW_H[0]) - (mv[1] ^ BMW_H[1]) + BMW_H[8] - BMW_H[10] + BMW_H[13];
    q[3] = (SHR(tmp, 2) ^ SHL(tmp, 2) ^ SPH_ROTL64(tmp, 28) ^ SPH_ROTL64(tmp, 59)) + BMW_H[4];
    tmp = (mv[1] ^ BMW_H[1]) + (mv[2] ^ BMW_H[2]) + BMW_H[9] - BMW_H[11] - BMW_H[14];
    q[4] = (SHR(tmp, 1) ^ tmp) + BMW_H[5];
    tmp = (mv[3] ^ BMW_H[3]) - (mv[2] ^ BMW_H[2]) + BMW_H[10] - BMW_H[12] + (mv[15] ^ BMW_H[15]);
    q[5] = (SHR(tmp, 1) ^ SHL(tmp, 3) ^ SPH_ROTL64(tmp, 4) ^ SPH_ROTL64(tmp, 37)) + BMW_H[6];
    tmp = (mv[4] ^ BMW_H[4]) - (mv[0] ^ BMW_H[0]) - (mv[3] ^ BMW_H[3]) - BMW_H[11] + BMW_H[13];
    q[6] = (SHR(tmp, 1) ^ SHL(tmp, 2) ^ SPH_ROTL64(tmp, 13) ^ SPH_ROTL64(tmp, 43)) + BMW_H[7];
    tmp = (mv[1] ^ BMW_H[1]) - (mv[4] ^ BMW_H[4]) - BMW_H[5] - BMW_H[12] - BMW_H[14];
    q[7] = (SHR(tmp, 2) ^ SHL(tmp, 1) ^ SPH_ROTL64(tmp, 19) ^ SPH_ROTL64(tmp, 53)) + BMW_H[8];
    tmp = (mv[2] ^ BMW_H[2]) - BMW_H[5] - BMW_H[6] + BMW_H[13] - (mv[15] ^ BMW_H[15]);
    q[8] = (SHR(tmp, 2) ^ SHL(tmp, 2) ^ SPH_ROTL64(tmp, 28) ^ SPH_ROTL64(tmp, 59)) + BMW_H[9];
    tmp = (mv[0] ^ BMW_H[0]) - (mv[3] ^ BMW_H[3]) + BMW_H[6] - BMW_H[7] + BMW_H[14];
    q[9] = (SHR(tmp, 1) ^ tmp) + BMW_H[10];
    tmp = BMW_H[8] - (mv[1] ^ BMW_H[1]) - (mv[4] ^ BMW_H[4]) - BMW_H[7] + (mv[15] ^ BMW_H[15]);
    q[10] = (SHR(tmp, 1) ^ SHL(tmp, 3) ^ SPH_ROTL64(tmp, 4) ^ SPH_ROTL64(tmp, 37)) + BMW_H[11];
    tmp = BMW_H[8] - (mv[0] ^ BMW_H[0]) - (mv[2] ^ BMW_H[2]) - BMW_H[5] + BMW_H[9];
    q[11] = (SHR(tmp, 1) ^ SHL(tmp, 2) ^ SPH_ROTL64(tmp, 13) ^ SPH_ROTL64(tmp, 43)) + BMW_H[12];
    tmp = (mv[1] ^ BMW_H[1]) + (mv[3] ^ BMW_H[3]) - BMW_H[6] - BMW_H[9] + BMW_H[10];
    q[12] = (SHR(tmp, 2) ^ SHL(tmp, 1) ^ SPH_ROTL64(tmp, 19) ^ SPH_ROTL64(tmp, 53)) + BMW_H[13];
    tmp = (mv[2] ^ BMW_H[2]) + (mv[4] ^ BMW_H[4]) + BMW_H[7] + BMW_H[10] + BMW_H[11];
    q[13] = (SHR(tmp, 2) ^ SHL(tmp, 2) ^ SPH_ROTL64(tmp, 28) ^ SPH_ROTL64(tmp, 59)) + BMW_H[14];
    tmp = (mv[3] ^ BMW_H[3]) - BMW_H[5] + BMW_H[8] - BMW_H[11] - BMW_H[12];
    q[14] = (SHR(tmp, 1) ^ tmp) + BMW_H[15];
    tmp = BMW_H[12] - (mv[4] ^ BMW_H[4]) - BMW_H[6] - BMW_H[9] + BMW_H[13];
    q[15] = (SHR(tmp, 1) ^ SHL(tmp, 3) ^ SPH_ROTL64(tmp, 4) ^ SPH_ROTL64(tmp, 37)) + BMW_H[0];

    //#pragma unroll 2
    for(int i=0;i<2;i++)
    {
        q[i+16] = 
        (SHR(q[i   ], 1) ^ SHL(q[i   ], 2) ^ SPH_ROTL64(q[i   ], 13) ^ SPH_ROTL64(q[i   ], 43)) +
        (SHR(q[i+ 1], 2) ^ SHL(q[i+ 1], 1) ^ SPH_ROTL64(q[i+ 1], 19) ^ SPH_ROTL64(q[i+ 1], 53)) +
        (SHR(q[i+ 2], 2) ^ SHL(q[i+ 2], 2) ^ SPH_ROTL64(q[i+ 2], 28) ^ SPH_ROTL64(q[i+ 2], 59)) +
        (SHR(q[i+ 3], 1) ^ SHL(q[i+ 3], 3) ^ SPH_ROTL64(q[i+ 3],  4) ^ SPH_ROTL64(q[i+ 3], 37)) +
        (SHR(q[i+ 4], 1) ^ SHL(q[i+ 4], 2) ^ SPH_ROTL64(q[i+ 4], 13) ^ SPH_ROTL64(q[i+ 4], 43)) +
        (SHR(q[i+ 5], 2) ^ SHL(q[i+ 5], 1) ^ SPH_ROTL64(q[i+ 5], 19) ^ SPH_ROTL64(q[i+ 5], 53)) +
        (SHR(q[i+ 6], 2) ^ SHL(q[i+ 6], 2) ^ SPH_ROTL64(q[i+ 6], 28) ^ SPH_ROTL64(q[i+ 6], 59)) +
        (SHR(q[i+ 7], 1) ^ SHL(q[i+ 7], 3) ^ SPH_ROTL64(q[i+ 7],  4) ^ SPH_ROTL64(q[i+ 7], 37)) +
        (SHR(q[i+ 8], 1) ^ SHL(q[i+ 8], 2) ^ SPH_ROTL64(q[i+ 8], 13) ^ SPH_ROTL64(q[i+ 8], 43)) +
        (SHR(q[i+ 9], 2) ^ SHL(q[i+ 9], 1) ^ SPH_ROTL64(q[i+ 9], 19) ^ SPH_ROTL64(q[i+ 9], 53)) +
        (SHR(q[i+10], 2) ^ SHL(q[i+10], 2) ^ SPH_ROTL64(q[i+10], 28) ^ SPH_ROTL64(q[i+10], 59)) +
        (SHR(q[i+11], 1) ^ SHL(q[i+11], 3) ^ SPH_ROTL64(q[i+11],  4) ^ SPH_ROTL64(q[i+11], 37)) +
        (SHR(q[i+12], 1) ^ SHL(q[i+12], 2) ^ SPH_ROTL64(q[i+12], 13) ^ SPH_ROTL64(q[i+12], 43)) +
        (SHR(q[i+13], 2) ^ SHL(q[i+13], 1) ^ SPH_ROTL64(q[i+13], 19) ^ SPH_ROTL64(q[i+13], 53)) +
        (SHR(q[i+14], 2) ^ SHL(q[i+14], 2) ^ SPH_ROTL64(q[i+14], 28) ^ SPH_ROTL64(q[i+14], 59)) +
        (SHR(q[i+15], 1) ^ SHL(q[i+15], 3) ^ SPH_ROTL64(q[i+15],  4) ^ SPH_ROTL64(q[i+15], 37)) +
        (( ((i+16)*(0x0555555555555555)) + SPH_ROTL64(mv[i], i+1) + SPH_ROTL64(mv[i+3], i+4) ) ^ BMW_H[i+7]);
    }

    //#pragma unroll 4
    for(int i=2;i<6;i++)
    {
        q[i+16] = CONST_EXP2 +
        (( ((i+16)*(0x0555555555555555)) + SPH_ROTL64(mv[i], i+1) - SPH_ROTL64(mv[i+10], i+11) ) ^ BMW_H[i+7]);
    }

    //#pragma unroll 3
    for(int i=6;i<9;i++)
    {
        q[i+16] = CONST_EXP2 +
        (( ((i+16)*(0x0555555555555555)) - SPH_ROTL64(mv[i-6], (i-6)+1) ) ^ BMW_H[i+7]);
    }

    //#pragma unroll 4
    for(int i=9;i<13;i++)
    {
        q[i+16] = CONST_EXP2 +
        (( ((i+16)*(0x0555555555555555)) +
        SPH_ROTL64(mv[i+3], i+4) - SPH_ROTL64(mv[i-6], (i-6)+1) ) ^ BMW_H[i-9]);
    }

    //#pragma unroll 3
    for(int i=13;i<16;i++)
    {
        q[i+16] = CONST_EXP2 +
        (( ((i+16)*(0x0555555555555555)) + SPH_ROTL64(mv[i], i+1) +
        SPH_ROTL64(mv[i-13], (i-13)+1) ) ^ BMW_H[i-9]);
    }

    sph_u64 XL64 = q[16]^q[17]^q[18]^q[19]^q[20]^q[21]^q[22]^q[23];
    sph_u64 XH64 = XL64^q[24]^q[25]^q[26]^q[27]^q[28]^q[29]^q[30]^q[31];

    BMW_H[0] = (SHL(XH64, 5) ^ SHR(q[16],5) ^ mv[0]) + ( XL64 ^ q[24] ^ q[0]);
    BMW_H[1] = (SHR(XH64, 7) ^ SHL(q[17],8) ^ mv[1]) + ( XL64 ^ q[25] ^ q[1]);
    BMW_H[2] = (SHR(XH64, 5) ^ SHL(q[18],5) ^ mv[2]) + ( XL64 ^ q[26] ^ q[2]);
    BMW_H[3] = (SHR(XH64, 1) ^ SHL(q[19],5) ^ mv[3]) + ( XL64 ^ q[27] ^ q[3]);
    BMW_H[4] = (SHR(XH64, 3) ^ q[20] ^ mv[4]) + ( XL64 ^ q[28] ^ q[4]);
    BMW_H[5] = (SHL(XH64, 6) ^ SHR(q[21],6)) + ( XL64 ^ q[29] ^ q[5]);
    BMW_H[6] = (SHR(XH64, 4) ^ SHL(q[22],6)) + ( XL64 ^ q[30] ^ q[6]);
    BMW_H[7] = (SHR(XH64,11) ^ SHL(q[23],2)) + ( XL64 ^ q[31] ^ q[7]);

    BMW_H[8] = SPH_ROTL64(BMW_H[4], 9) + ( XH64 ^ q[24]) + (SHL(XL64,8) ^ q[23] ^ q[8]);
    BMW_H[9] = SPH_ROTL64(BMW_H[5],10) + ( XH64 ^ q[25]) + (SHR(XL64,6) ^ q[16] ^ q[9]);
    BMW_H[10] = SPH_ROTL64(BMW_H[6],11) + ( XH64 ^ q[26]) + (SHL(XL64,6) ^ q[17] ^ q[10]);
    BMW_H[11] = SPH_ROTL64(BMW_H[7],12) + ( XH64 ^ q[27]) + (SHL(XL64,4) ^ q[18] ^ q[11]);
    BMW_H[12] = SPH_ROTL64(BMW_H[0],13) + ( XH64 ^ q[28]) + (SHR(XL64,3) ^ q[19] ^ q[12]);
    BMW_H[13] = SPH_ROTL64(BMW_H[1],14) + ( XH64 ^ q[29]) + (SHR(XL64,4) ^ q[20] ^ q[13]);
    BMW_H[14] = SPH_ROTL64(BMW_H[2],15) + ( XH64 ^ q[30]) + (SHR(XL64,7) ^ q[21] ^ q[14]);
    BMW_H[15] = SPH_ROTL64(BMW_H[3],16) + ( XH64 ^ q[31] ^ mv[15]) + (SHR(XL64,2) ^ q[22] ^ q[15]);

    //#pragma unroll 16
    for(int i=0;i<16;i++)
    {
        mv[i] = BMW_H[i];
        BMW_H[i] = 0xaaaaaaaaaaaaaaa0 + (sph_u64)i;
    }

    tmp = (mv[5] ^ BMW_H[5]) - (mv[7] ^ BMW_H[7]) + (mv[10] ^ BMW_H[10]) + (mv[13] ^ BMW_H[13]) + (mv[14] ^ BMW_H[14]);
    q[0] = (SHR(tmp, 1) ^ SHL(tmp, 3) ^ SPH_ROTL64(tmp, 4) ^ SPH_ROTL64(tmp, 37)) + BMW_H[1];
    tmp = (mv[6] ^ BMW_H[6]) - (mv[8] ^ BMW_H[8]) + (mv[11] ^ BMW_H[11]) + (mv[14] ^ BMW_H[14]) - (mv[15] ^ BMW_H[15]);
    q[1] = (SHR(tmp, 1) ^ SHL(tmp, 2) ^ SPH_ROTL64(tmp, 13) ^ SPH_ROTL64(tmp, 43)) + BMW_H[2];
    tmp = (mv[0] ^ BMW_H[0]) + (mv[7] ^ BMW_H[7]) + (mv[9] ^ BMW_H[9]) - (mv[12] ^ BMW_H[12]) + (mv[15] ^ BMW_H[15]);
    q[2] = (SHR(tmp, 2) ^ SHL(tmp, 1) ^ SPH_ROTL64(tmp, 19) ^ SPH_ROTL64(tmp, 53)) + BMW_H[3];
    tmp = (mv[0] ^ BMW_H[0]) - (mv[1] ^ BMW_H[1]) + (mv[8] ^ BMW_H[8]) - (mv[10] ^ BMW_H[10]) + (mv[13] ^ BMW_H[13]);
    q[3] = (SHR(tmp, 2) ^ SHL(tmp, 2) ^ SPH_ROTL64(tmp, 28) ^ SPH_ROTL64(tmp, 59)) + BMW_H[4];
    tmp = (mv[1] ^ BMW_H[1]) + (mv[2] ^ BMW_H[2]) + (mv[9] ^ BMW_H[9]) - (mv[11] ^ BMW_H[11]) - (mv[14] ^ BMW_H[14]);
    q[4] = (SHR(tmp, 1) ^ tmp) + BMW_H[5];
    tmp = (mv[3] ^ BMW_H[3]) - (mv[2] ^ BMW_H[2]) + (mv[10] ^ BMW_H[10]) - (mv[12] ^ BMW_H[12]) + (mv[15] ^ BMW_H[15]);
    q[5] = (SHR(tmp, 1) ^ SHL(tmp, 3) ^ SPH_ROTL64(tmp, 4) ^ SPH_ROTL64(tmp, 37)) + BMW_H[6];
    tmp = (mv[4] ^ BMW_H[4]) - (mv[0] ^ BMW_H[0]) - (mv[3] ^ BMW_H[3]) - (mv[11] ^ BMW_H[11]) + (mv[13] ^ BMW_H[13]);
    q[6] = (SHR(tmp, 1) ^ SHL(tmp, 2) ^ SPH_ROTL64(tmp, 13) ^ SPH_ROTL64(tmp, 43)) + BMW_H[7];
    tmp = (mv[1] ^ BMW_H[1]) - (mv[4] ^ BMW_H[4]) - (mv[5] ^ BMW_H[5]) - (mv[12] ^ BMW_H[12]) - (mv[14] ^ BMW_H[14]);
    q[7] = (SHR(tmp, 2) ^ SHL(tmp, 1) ^ SPH_ROTL64(tmp, 19) ^ SPH_ROTL64(tmp, 53)) + BMW_H[8];
    tmp = (mv[2] ^ BMW_H[2]) - (mv[5] ^ BMW_H[5]) - (mv[6] ^ BMW_H[6]) + (mv[13] ^ BMW_H[13]) - (mv[15] ^ BMW_H[15]);
    q[8] = (SHR(tmp, 2) ^ SHL(tmp, 2) ^ SPH_ROTL64(tmp, 28) ^ SPH_ROTL64(tmp, 59)) + BMW_H[9];
    tmp = (mv[0] ^ BMW_H[0]) - (mv[3] ^ BMW_H[3]) + (mv[6] ^ BMW_H[6]) - (mv[7] ^ BMW_H[7]) + (mv[14] ^ BMW_H[14]);
    q[9] = (SHR(tmp, 1) ^ tmp) + BMW_H[10];
    tmp = (mv[8] ^ BMW_H[8]) - (mv[1] ^ BMW_H[1]) - (mv[4] ^ BMW_H[4]) - (mv[7] ^ BMW_H[7]) + (mv[15] ^ BMW_H[15]);
    q[10] = (SHR(tmp, 1) ^ SHL(tmp, 3) ^ SPH_ROTL64(tmp, 4) ^ SPH_ROTL64(tmp, 37)) + BMW_H[11];
    tmp = (mv[8] ^ BMW_H[8]) - (mv[0] ^ BMW_H[0]) - (mv[2] ^ BMW_H[2]) - (mv[5] ^ BMW_H[5]) + (mv[9] ^ BMW_H[9]);
    q[11] = (SHR(tmp, 1) ^ SHL(tmp, 2) ^ SPH_ROTL64(tmp, 13) ^ SPH_ROTL64(tmp, 43)) + BMW_H[12];
    tmp = (mv[1] ^ BMW_H[1]) + (mv[3] ^ BMW_H[3]) - (mv[6] ^ BMW_H[6]) - (mv[9] ^ BMW_H[9]) + (mv[10] ^ BMW_H[10]);
    q[12] = (SHR(tmp, 2) ^ SHL(tmp, 1) ^ SPH_ROTL64(tmp, 19) ^ SPH_ROTL64(tmp, 53)) + BMW_H[13];
    tmp = (mv[2] ^ BMW_H[2]) + (mv[4] ^ BMW_H[4]) + (mv[7] ^ BMW_H[7]) + (mv[10] ^ BMW_H[10]) + (mv[11] ^ BMW_H[11]);
    q[13] = (SHR(tmp, 2) ^ SHL(tmp, 2) ^ SPH_ROTL64(tmp, 28) ^ SPH_ROTL64(tmp, 59)) + BMW_H[14];
    tmp = (mv[3] ^ BMW_H[3]) - (mv[5] ^ BMW_H[5]) + (mv[8] ^ BMW_H[8]) - (mv[11] ^ BMW_H[11]) - (mv[12] ^ BMW_H[12]);
    q[14] = (SHR(tmp, 1) ^ tmp) + BMW_H[15];
    tmp = (mv[12] ^ BMW_H[12]) - (mv[4] ^ BMW_H[4]) - (mv[6] ^ BMW_H[6]) - (mv[9] ^ BMW_H[9]) + (mv[13] ^ BMW_H[13]);
    q[15] = (SHR(tmp, 1) ^ SHL(tmp, 3) ^ SPH_ROTL64(tmp, 4) ^ SPH_ROTL64(tmp, 37)) + BMW_H[0];

    //#pragma unroll 2
    for(int i=0;i<2;i++)
    {
        q[i+16] =
        (SHR(q[i], 1) ^ SHL(q[i], 2) ^ SPH_ROTL64(q[i], 13) ^ SPH_ROTL64(q[i], 43)) +
        (SHR(q[i+1], 2) ^ SHL(q[i+1], 1) ^ SPH_ROTL64(q[i+1], 19) ^ SPH_ROTL64(q[i+1], 53)) +
        (SHR(q[i+2], 2) ^ SHL(q[i+2], 2) ^ SPH_ROTL64(q[i+2], 28) ^ SPH_ROTL64(q[i+2], 59)) +
        (SHR(q[i+3], 1) ^ SHL(q[i+3], 3) ^ SPH_ROTL64(q[i+3], 4) ^ SPH_ROTL64(q[i+3], 37)) +
        (SHR(q[i+4], 1) ^ SHL(q[i+4], 2) ^ SPH_ROTL64(q[i+4], 13) ^ SPH_ROTL64(q[i+4], 43)) +
        (SHR(q[i+5], 2) ^ SHL(q[i+5], 1) ^ SPH_ROTL64(q[i+5], 19) ^ SPH_ROTL64(q[i+5], 53)) +
        (SHR(q[i+6], 2) ^ SHL(q[i+6], 2) ^ SPH_ROTL64(q[i+6], 28) ^ SPH_ROTL64(q[i+6], 59)) +
        (SHR(q[i+7], 1) ^ SHL(q[i+7], 3) ^ SPH_ROTL64(q[i+7], 4) ^ SPH_ROTL64(q[i+7], 37)) +
        (SHR(q[i+8], 1) ^ SHL(q[i+8], 2) ^ SPH_ROTL64(q[i+8], 13) ^ SPH_ROTL64(q[i+8], 43)) +
        (SHR(q[i+9], 2) ^ SHL(q[i+9], 1) ^ SPH_ROTL64(q[i+9], 19) ^ SPH_ROTL64(q[i+9], 53)) +
        (SHR(q[i+10], 2) ^ SHL(q[i+10], 2) ^ SPH_ROTL64(q[i+10], 28) ^ SPH_ROTL64(q[i+10], 59)) +
        (SHR(q[i+11], 1) ^ SHL(q[i+11], 3) ^ SPH_ROTL64(q[i+11], 4) ^ SPH_ROTL64(q[i+11], 37)) +
        (SHR(q[i+12], 1) ^ SHL(q[i+12], 2) ^ SPH_ROTL64(q[i+12], 13) ^ SPH_ROTL64(q[i+12], 43)) +
        (SHR(q[i+13], 2) ^ SHL(q[i+13], 1) ^ SPH_ROTL64(q[i+13], 19) ^ SPH_ROTL64(q[i+13], 53)) +
        (SHR(q[i+14], 2) ^ SHL(q[i+14], 2) ^ SPH_ROTL64(q[i+14], 28) ^ SPH_ROTL64(q[i+14], 59)) +
        (SHR(q[i+15], 1) ^ SHL(q[i+15], 3) ^ SPH_ROTL64(q[i+15], 4) ^ SPH_ROTL64(q[i+15], 37)) +
        (( ((i+16)*(0x0555555555555555)) + SPH_ROTL64(mv[i], i+1) +
        SPH_ROTL64(mv[i+3], i+4) - SPH_ROTL64(mv[i+10], i+11) ) ^ BMW_H[i+7]);
    }

    //#pragma unroll 4
    for(int i=2;i<6;i++)
    {
        q[i+16] = CONST_EXP2 +
        (( ((i+16)*(0x0555555555555555)) + SPH_ROTL64(mv[i], i+1) +
        SPH_ROTL64(mv[i+3], i+4) - SPH_ROTL64(mv[i+10], i+11) ) ^ BMW_H[i+7]);
    }

    //#pragma unroll 3
    for(int i=6;i<9;i++)
    {
        q[i+16] = CONST_EXP2 +
        (( ((i+16)*(0x0555555555555555)) + SPH_ROTL64(mv[i], i+1) +
        SPH_ROTL64(mv[i+3], i+4) - SPH_ROTL64(mv[i-6], (i-6)+1) ) ^ BMW_H[i+7]);
    }

    //#pragma unroll 4
    for(int i=9;i<13;i++)
    {
        q[i+16] = CONST_EXP2 +
        (( ((i+16)*(0x0555555555555555)) + SPH_ROTL64(mv[i], i+1) +
        SPH_ROTL64(mv[i+3], i+4) - SPH_ROTL64(mv[i-6], (i-6)+1) ) ^ BMW_H[i-9]);
    }

    //#pragma unroll 3
    for(int i=13;i<16;i++)
    {
        q[i+16] = CONST_EXP2 +
        (( ((i+16)*(0x0555555555555555)) + SPH_ROTL64(mv[i], i+1) +
        SPH_ROTL64(mv[i-13], (i-13)+1) - SPH_ROTL64(mv[i-6], (i-6)+1) ) ^ BMW_H[i-9]);
    }

    XL64 = q[16]^q[17]^q[18]^q[19]^q[20]^q[21]^q[22]^q[23];
    XH64 = XL64^q[24]^q[25]^q[26]^q[27]^q[28]^q[29]^q[30]^q[31];
 
    BMW_H[4] = (SHR(XH64, 3) ^ q[20] ^ mv[4]) + ( XL64 ^ q[28] ^ q[4]);
    BMW_H[5] = (SHL(XH64, 6) ^ SHR(q[21],6) ^ mv[5]) + ( XL64 ^ q[29] ^ q[5]);
    BMW_H[6] = (SHR(XH64, 4) ^ SHL(q[22],6) ^ mv[6]) + ( XL64 ^ q[30] ^ q[6]);
    BMW_H[7] = (SHR(XH64,11) ^ SHL(q[23],2) ^ mv[7]) + ( XL64 ^ q[31] ^ q[7]);

    BMW_H[8] = SPH_ROTL64(BMW_H[4], 9) + ( XH64 ^ q[24] ^ mv[8]) + (SHL(XL64,8) ^ q[23] ^ q[8]);
    BMW_H[9] = SPH_ROTL64(BMW_H[5],10) + ( XH64 ^ q[25] ^ mv[9]) + (SHR(XL64,6) ^ q[16] ^ q[9]);
    BMW_H[10] = SPH_ROTL64(BMW_H[6],11) + ( XH64 ^ q[26] ^ mv[10]) + (SHL(XL64,6) ^ q[17] ^ q[10]);
    BMW_H[11] = SPH_ROTL64(BMW_H[7],12) + ( XH64 ^ q[27] ^ mv[11]) + (SHL(XL64,4) ^ q[18] ^ q[11]);

    hash->h8[0] = BMW_H[8];
    hash->h8[1] = BMW_H[9];
    hash->h8[2] = BMW_H[10];
    hash->h8[3] = BMW_H[11];
}

// groestl
void hash_x16rs_func_2(hash_32* ALIGN hash, __local const ulong* ALIGN T0, __local const ulong* ALIGN T1, __local const ulong* ALIGN T2, __local const ulong* ALIGN T3)
{
  ulong ALIGN M[16];
  ulong ALIGN G[16];
  M[0] = G[0] = hash->h8[0];
  M[1] = G[1] = hash->h8[1];
  M[2] = G[2] = hash->h8[2];
  M[3] = G[3] = hash->h8[3];

  ulong ALIGN H[16];
	
  H[0] = G[0]  ^ PC64(0  << 4, 0);
  H[1] = G[1]  ^ PC64(1  << 4, 0);
  H[2] = G[2]  ^ PC64(2  << 4, 0);
  H[3] = G[3]  ^ PC64(3  << 4, 0);
  G[0] = (T0[B64_0(H[0])] ^ T1[B64_1(H[1])] ^ T2[B64_2(H[2])] ^ T3[B64_3(H[3])] ^ 11945401302657479319);
  G[1] = (T0[B64_0(H[1])] ^ T1[B64_1(H[2])] ^ T2[B64_2(H[3])] ^ 5864639994750408448);
  G[2] = (T0[B64_0(H[2])] ^ T1[B64_1(H[3])] ^ 14310125096367509925);
  G[3] = (T0[B64_0(H[3])] ^ 7133757133233952611);
  G[4] = 3669252037894537041;
  G[5] = (230739945414771191 ^ as_ulong(as_uint2(T3[B64_7(H[0])]).s10));
  G[6] = (2210698400969090538 ^ as_ulong(as_uint2(T3[B64_7(H[1])]).s10));
  G[7] = (519542093484052467 ^ as_ulong(as_uint2(T3[B64_7(H[2])]).s10));
  G[8] = (2633073143201150672 ^ as_ulong(as_uint2(T3[B64_7(H[3])]).s10));
  G[9] = 14954345047970975975;
  G[10] = (as_ulong(as_uint2(T2[B64_6(H[0])]).s10) ^ 10015259800346600524);
  G[11] = (as_ulong(as_uint2(T1[B64_5(H[0])]).s10) ^ as_ulong(as_uint2(T2[B64_6(H[1])]).s10) ^ 8126327043163178471);
  G[12] = (as_ulong(as_uint2(T0[B64_4(H[0])]).s10) ^ as_ulong(as_uint2(T1[B64_5(H[1])]).s10) ^ as_ulong(as_uint2(T2[B64_6(H[2])]).s10) ^ 8058591818875624105);
  G[13] = (T3[B64_3(H[0])]  ^ as_ulong(as_uint2(T0[B64_4(H[1])]).s10) ^ as_ulong(as_uint2(T1[B64_5(H[2])]).s10) ^ as_ulong(as_uint2(T2[B64_6(H[3])]).s10) ^ 1456122202456506289);
  G[14] = (T2[B64_2(H[0])] ^ T3[B64_3(H[1])]  ^ as_ulong(as_uint2(T0[B64_4(H[2])]).s10) ^ as_ulong(as_uint2(T1[B64_5(H[3])]).s10) ^ 9849160040520025817);
  G[15] = (T1[B64_1(H[0])] ^ T2[B64_2(H[1])] ^ T3[B64_3(H[2])]  ^ as_ulong(as_uint2(T0[B64_4(H[3])]).s10) ^ 269551315503033440);
  
  H[0 ] = M[0] ^ QC64(0  << 4, 0);
  H[1 ] = M[1]  ^ QC64(1  << 4, 0);
  H[2 ] = M[2]  ^ QC64(2  << 4, 0);
  H[3 ] = M[3]  ^ QC64(3  << 4, 0);
  
  M[0] = (T0[B64_0(H[1])] ^ T1[B64_1(H[3])] ^ as_ulong(as_uint2(T0[B64_4(H[0])]).s10) ^ as_ulong(as_uint2(T1[B64_5(H[2])]).s10) ^ 15705510229074032155);
  M[1] = (T0[B64_0(H[2])] ^ as_ulong(as_uint2(T0[B64_4(H[1])]).s10) ^ as_ulong(as_uint2(T1[B64_5(H[3])]).s10) ^ 12170560161315398722);
  M[2] = (T0[B64_0(H[3])] ^ as_ulong(as_uint2(T0[B64_4(H[2])]).s10) ^ 13825302525975853362);
  M[3] = (as_ulong(as_uint2(T0[B64_4(H[3])]).s10) ^ 17601400094414895344);
  M[4] = 10633925005790587395;
  M[5] = (T3[B64_3(H[0])] ^ 5422291237815970993);
  M[6] = (T3[B64_3(H[1])] ^ 12882214474931762256);
  M[7] = (T3[B64_3(H[2])] ^ 8216379176195919723);
  M[8] = (T3[B64_3(H[3])] ^ 14102764044196225910);
  M[9] = 6583109234331108644;
  M[10] = (1591459941175030872 ^ as_ulong(as_uint2(T3[B64_7(H[0])]).s10));
  M[11] = (T2[B64_2(H[0])] ^ 5645326093933893730 ^ as_ulong(as_uint2(T3[B64_7(H[1])]).s10));
  M[12] = (T2[B64_2(H[1])] ^ 3203209520027427372 ^ as_ulong(as_uint2(T2[B64_6(H[0])]).s10) ^ as_ulong(as_uint2(T3[B64_7(H[2])]).s10));
  M[13] = (T1[B64_1(H[0])] ^ T2[B64_2(H[2])] ^ 1597748913811508736 ^ as_ulong(as_uint2(T2[B64_6(H[1])]).s10) ^ as_ulong(as_uint2(T3[B64_7(H[3])]).s10));
  M[14] = (T1[B64_1(H[1])] ^ T2[B64_2(H[3])] ^ as_ulong(as_uint2(T1[B64_5(H[0])]).s10) ^ as_ulong(as_uint2(T2[B64_6(H[2])]).s10) ^ 5214724303542560770);
  M[15] = (T0[B64_0(H[0])] ^ T1[B64_1(H[2])] ^ as_ulong(as_uint2(T1[B64_5(H[1])]).s10) ^ as_ulong(as_uint2(T2[B64_6(H[3])]).s10) ^ 16053254594141210712);

  #pragma nounroll
	for (int i = 1; i < 14; ++i)
	{
    ulong rc[16];
    #pragma unroll 16
    for (int j = 0; j < 16; j++)
        rc[j] = PC64(j << 4, i);
    #pragma unroll 16
    for (int j = 0; j < 16; j++)
        H[j] = G[j] ^ rc[j];

    G[0]  = groestl_rbtt(H, 0, 1, 2, 3, 4, 5, 6, 11, T0, T1, T2, T3);
    G[1]  = groestl_rbtt(H, 1, 2, 3, 4, 5, 6, 7, 12, T0, T1, T2, T3);
    G[2]  = groestl_rbtt(H, 2, 3, 4, 5, 6, 7, 8, 13, T0, T1, T2, T3);
    G[3]  = groestl_rbtt(H, 3, 4, 5, 6, 7, 8, 9, 14, T0, T1, T2, T3);
    G[4]  = groestl_rbtt(H, 4, 5, 6, 7, 8, 9, 10, 15, T0, T1, T2, T3);
    G[5]  = groestl_rbtt(H, 5, 6, 7, 8, 9, 10, 11, 0, T0, T1, T2, T3);
    G[6]  = groestl_rbtt(H, 6, 7, 8, 9, 10, 11, 12, 1, T0, T1, T2, T3);
    G[7]  = groestl_rbtt(H, 7, 8, 9, 10, 11, 12, 13, 2, T0, T1, T2, T3);
    G[8]  = groestl_rbtt(H, 8, 9, 10, 11, 12, 13, 14, 3, T0, T1, T2, T3);
    G[9]  = groestl_rbtt(H, 9, 10, 11, 12, 13, 14, 15, 4, T0, T1, T2, T3);
    G[10] = groestl_rbtt(H, 10, 11, 12, 13, 14, 15, 0, 5, T0, T1, T2, T3);
    G[11] = groestl_rbtt(H, 11, 12, 13, 14, 15, 0, 1, 6, T0, T1, T2, T3);
    G[12] = groestl_rbtt(H, 12, 13, 14, 15, 0, 1, 2, 7, T0, T1, T2, T3);
    G[13] = groestl_rbtt(H, 13, 14, 15, 0, 1, 2, 3, 8, T0, T1, T2, T3);
    G[14] = groestl_rbtt(H, 14, 15, 0, 1, 2, 3, 4, 9, T0, T1, T2, T3);
    G[15] = groestl_rbtt(H, 15, 0, 1, 2, 3, 4, 5, 10, T0, T1, T2, T3);
  }
  
  #pragma nounroll
  for(int i = 1; i < 13; ++i)
	{
    ulong rc[16];
    #pragma unroll 16
    for (int j = 0; j < 16; j++)
        rc[j] = QC64(j << 4, i);
    #pragma unroll 16
    for (int j = 0; j < 16; j++)
        H[j] = M[j] ^ rc[j];

    M[0]  = groestl_rbtt(H, 1, 3, 5, 11, 0, 2, 4, 6, T0, T1, T2, T3);
    M[1]  = groestl_rbtt(H, 2, 4, 6, 12, 1, 3, 5, 7, T0, T1, T2, T3);
    M[2]  = groestl_rbtt(H, 3, 5, 7, 13, 2, 4, 6, 8, T0, T1, T2, T3);
    M[3]  = groestl_rbtt(H, 4, 6, 8, 14, 3, 5, 7, 9, T0, T1, T2, T3);
    M[4]  = groestl_rbtt(H, 5, 7, 9, 15, 4, 6, 8, 10, T0, T1, T2, T3);
    M[5]  = groestl_rbtt(H, 6, 8, 10, 0, 5, 7, 9, 11, T0, T1, T2, T3);
    M[6]  = groestl_rbtt(H, 7, 9, 11, 1, 6, 8, 10, 12, T0, T1, T2, T3);
    M[7]  = groestl_rbtt(H, 8, 10, 12, 2, 7, 9, 11, 13, T0, T1, T2, T3);
    M[8]  = groestl_rbtt(H, 9, 11, 13, 3, 8, 10, 12, 14, T0, T1, T2, T3);
    M[9]  = groestl_rbtt(H, 10, 12, 14, 4, 9, 11, 13, 15, T0, T1, T2, T3);
    M[10] = groestl_rbtt(H, 11, 13, 15, 5, 10, 12, 14, 0, T0, T1, T2, T3);
    M[11] = groestl_rbtt(H, 12, 14, 0, 6, 11, 13, 15, 1, T0, T1, T2, T3);
    M[12] = groestl_rbtt(H, 13, 15, 1, 7, 12, 14, 0, 2, T0, T1, T2, T3);
    M[13] = groestl_rbtt(H, 14, 0, 2, 8, 13, 15, 1, 3, T0, T1, T2, T3);
    M[14] = groestl_rbtt(H, 15, 1, 3, 9, 14, 0, 2, 4, T0, T1, T2, T3);
    M[15] = groestl_rbtt(H, 0, 2, 4, 10, 15, 1, 3, 5, T0, T1, T2, T3);
	}

  #pragma unroll 16
  for (int j = 0; j < 16; j++)
    H[j] = M[j] ^ QC64(j << 4, 13);
  
  groestl_rbtt_last(&G[0], H, 1, 3, 5, 11, 0, 2, 4, 6, T0, T1, T2, T3);
  groestl_rbtt_last(&G[1], H, 2, 4, 6, 12, 1, 3, 5, 7, T0, T1, T2, T3);
  groestl_rbtt_last(&G[2], H, 3, 5, 7, 13, 2, 4, 6, 8, T0, T1, T2, T3);
  groestl_rbtt_last(&G[3], H, 4, 6, 8, 14, 3, 5, 7, 9, T0, T1, T2, T3);
  groestl_rbtt_last(&G[4], H, 5, 7, 9, 15, 4, 6, 8, 10, T0, T1, T2, T3);
  groestl_rbtt_last(&G[5], H, 6, 8, 10, 0, 5, 7, 9, 11, T0, T1, T2, T3);
  groestl_rbtt_last(&G[6], H, 7, 9, 11, 1, 6, 8, 10, 12, T0, T1, T2, T3);
  groestl_rbtt_last(&G[7], H, 8, 10, 12, 2, 7, 9, 11, 13, T0, T1, T2, T3);
  groestl_rbtt_last(&G[8], H, 9, 11, 13, 3, 8, 10, 12, 14, T0, T1, T2, T3);
  groestl_rbtt_last(&G[9], H, 10, 12, 14, 4, 9, 11, 13, 15, T0, T1, T2, T3);
  groestl_rbtt_last(&G[10], H, 11, 13, 15, 5, 10, 12, 14, 0, T0, T1, T2, T3);
  groestl_rbtt_last(&G[11], H, 12, 14, 0, 6, 11, 13, 15, 1, T0, T1, T2, T3);
  groestl_rbtt_last(&G[12], H, 13, 15, 1, 7, 12, 14, 0, 2, T0, T1, T2, T3);
  groestl_rbtt_last(&G[13], H, 14, 0, 2, 8, 13, 15, 1, 3, T0, T1, T2, T3);
  groestl_rbtt_last(&G[14], H, 15, 1, 3, 9, 14, 0, 2, 4, T0, T1, T2, T3);
  groestl_rbtt_last(&G[15], H, 0, 2, 4, 10, 15, 1, 3, 5, T0, T1, T2, T3);

	G[15] ^= 0x0002000000000000UL;
	
  hash->h8[0]  = G[8];
  hash->h8[1]  = G[9];
  hash->h8[2]  = G[10];
  hash->h8[3]  = G[11];
	
  #pragma nounroll
	for(int i = 0; i < 13; ++i)
	{
    ulong rc[16];
    #pragma unroll 16
    for (int j = 0; j < 16; j++)
        rc[j] = PC64(j << 4, i);
    #pragma unroll 16
    for (int j = 0; j < 16; j++)
        H[j] = G[j] ^ rc[j];
			
    G[0]  = groestl_rbtt(H, 0, 1, 2, 3, 4, 5, 6, 11, T0, T1, T2, T3);
    G[1]  = groestl_rbtt(H, 1, 2, 3, 4, 5, 6, 7, 12, T0, T1, T2, T3);
    G[3]  = groestl_rbtt(H, 3, 4, 5, 6, 7, 8, 9, 14, T0, T1, T2, T3);
    G[4]  = groestl_rbtt(H, 4, 5, 6, 7, 8, 9, 10, 15, T0, T1, T2, T3);
    G[5]  = groestl_rbtt(H, 5, 6, 7, 8, 9, 10, 11, 0, T0, T1, T2, T3);
    G[6]  = groestl_rbtt(H, 6, 7, 8, 9, 10, 11, 12, 1, T0, T1, T2, T3);
    G[8]  = groestl_rbtt(H, 8, 9, 10, 11, 12, 13, 14, 3, T0, T1, T2, T3);
    G[9]  = groestl_rbtt(H, 9, 10, 11, 12, 13, 14, 15, 4, T0, T1, T2, T3);
    G[10] = groestl_rbtt(H, 10, 11, 12, 13, 14, 15, 0, 5, T0, T1, T2, T3);
    G[11] = groestl_rbtt(H, 11, 12, 13, 14, 15, 0, 1, 6, T0, T1, T2, T3);
    G[12] = groestl_rbtt(H, 12, 13, 14, 15, 0, 1, 2, 7, T0, T1, T2, T3);
    G[13] = groestl_rbtt(H, 13, 14, 15, 0, 1, 2, 3, 8, T0, T1, T2, T3);
    G[14] = groestl_rbtt(H, 14, 15, 0, 1, 2, 3, 4, 9, T0, T1, T2, T3);
    G[15] = groestl_rbtt(H, 15, 0, 1, 2, 3, 4, 5, 10, T0, T1, T2, T3);
    
		if(i < 12) {
      G[2]  = groestl_rbtt(H, 2, 3, 4, 5, 6, 7, 8, 13, T0, T1, T2, T3);
      G[7]  = groestl_rbtt(H, 7, 8, 9, 10, 11, 12, 13, 2, T0, T1, T2, T3);
    }
	}

  #pragma unroll 16
  for (int j = 0; j < 16; j++)
    H[j] = G[j] ^ PC64(j << 4, 13);

  groestl_rbtt_last(&hash->h8[0], H, 8, 9, 10, 11, 12, 13, 14, 3, T0, T1, T2, T3);
  groestl_rbtt_last(&hash->h8[1], H, 9, 10, 11, 12, 13, 14, 15, 4, T0, T1, T2, T3);
  groestl_rbtt_last(&hash->h8[2], H, 10, 11, 12, 13, 14, 15, 0, 5, T0, T1, T2, T3);
  groestl_rbtt_last(&hash->h8[3], H, 11, 12, 13, 14, 15, 0, 1, 6, T0, T1, T2, T3);
}

// jh
void hash_x16rs_func_3(hash_32* hash)
{
    sph_u64 h0h = C64e(0x6fd14b963e00aa17) ^ hash->h8[0], h0l = C64e(0x636a2e057a15d543) ^ hash->h8[1], h1h = C64e(0x8a225e8d0c97ef0b) ^ hash->h8[2], h1l = C64e(0xe9341259f2b3c361) ^ hash->h8[3], h2h = C64e(0x891da0c1536f801e) ^ SPH_C64(0x0000000000000080), h2l = C64e(0x2aa9056bea2b6d80), h3h = C64e(0x588eccdb2075baa6), h3l = C64e(0xa90f3a76baf83bf7);
    sph_u64 h4h = C64e(0x0169e60541e34a69), h4l = C64e(0x46b58a8e2e6fe65a), h5h = C64e(0x1047a7d0c1843c24), h5l = C64e(0x3b6e71b12d5ac199), h6h = C64e(0xcf57f6ec9db1f856), h6l = C64e(0xa706887c5716b156), h7h = C64e(0xe3c2fcdfe68517fb), h7l = C64e(0x545a4678cc8cdd4b);
    sph_u64 tmp;

    E8;
    h4h ^= hash->h8[0];
    h4l ^= hash->h8[1];
    h5h ^= hash->h8[2];
    h5l ^= hash->h8[3];
    h6h ^= SPH_C64(0x0000000000000080);

    h3l ^= SPH_C64(0x0001000000000000);
    E8;

    hash->h8[0] = h4h;
    hash->h8[1] = h4l;
    hash->h8[2] = h5h;
    hash->h8[3] = h5l;
}

// keccak
void hash_x16rs_func_4(hash_32* hash)
{
    sph_u64 a00 = hash->h8[0], a01 = 0, a02 = 0, a03 = 0, a04 = SPH_C64(0xFFFFFFFFFFFFFFFF);
    sph_u64 a10 = SPH_C64(0xFFFFFFFFFFFFFFFF) ^ hash->h8[1], a11 = 0, a12 = 0, a13 = 0, a14 = 0;
    sph_u64 a20 = SPH_C64(0xFFFFFFFFFFFFFFFF) ^ hash->h8[2], a21 = 0, a22 = SPH_C64(0xFFFFFFFFFFFFFFFF), a23 = SPH_C64(0xFFFFFFFFFFFFFFFF), a24 = 0;
    sph_u64 a30 = hash->h8[3], a31 = SPH_C64(0xFFFFFFFFFFFFFFFF) ^ 0x8000000000000000, a32 = 0, a33 = 0, a34 = 0;
    sph_u64 a40 = SPH_C64(0x000000000000001), a41 = 0, a42 = 0, a43 = 0, a44 = 0;

    KECCAK_F_1600;

    hash->h8[0] = a00;
    hash->h8[1] = ~a10;
    hash->h8[2] = ~a20;
    hash->h8[3] = a30;
}

// skein
void hash_x16rs_func_5(hash_32* hash)
{
    // skein
    sph_u64 h0 = SPH_C64(0x4903ADFF749C51CE), h1 = SPH_C64(0x0D95DE399746DF03), 
    h2 = SPH_C64(0x8FD1934127C79BCE), h3 = SPH_C64(0x9A255629FF352CB1), 
    h4 = SPH_C64(0x5DB62599DF6CA7B0), h5 = SPH_C64(0xEABE394CA9D5C3F4), 
    h6 = SPH_C64(0x991112C71A75B523), h7 = SPH_C64(0xAE18A40B660FCC33);
    sph_u64 m0, m1, m2, m3, m4, m5, m6, m7;
    sph_u64 bcount = 0;

    m0 = hash->h8[0];
    m1 = hash->h8[1];
    m2 = hash->h8[2];
    m3 = hash->h8[3];
    m4 = 0;
    m5 = 0;
    m6 = 0;
    m7 = 0;

    UBI_BIG;

    bcount = 0;
    m0 = 0;
    m1 = 0;
    m2 = 0;
    m3 = 0;
    m4 = 0;
    m5 = 0;
    m6 = 0;
    m7 = 0;

    UBI_BIG_LAST;

    hash->h8[0] = h0;
    hash->h8[1] = h1;
    hash->h8[2] = h2;
    hash->h8[3] = h3;
}

// luffa
void hash_x16rs_func_6(hash_32* hash)
{
    sph_u32 V00 = SPH_C32(0x6d251e69), V01 = SPH_C32(0x44b051e0), V02 = SPH_C32(0x4eaa6fb4), V03 = SPH_C32(0xdbf78465), V04 = SPH_C32(0x6e292011), V05 = SPH_C32(0x90152df4), V06 = SPH_C32(0xee058139), V07 = SPH_C32(0xdef610bb);
    sph_u32 V10 = SPH_C32(0xc3b44b95), V11 = SPH_C32(0xd9d2f256), V12 = SPH_C32(0x70eee9a0), V13 = SPH_C32(0xde099fa3), V14 = SPH_C32(0x5d9b0557), V15 = SPH_C32(0x8fc944b3), V16 = SPH_C32(0xcf1ccf0e), V17 = SPH_C32(0x746cd581);
    sph_u32 V20 = SPH_C32(0xf7efc89d), V21 = SPH_C32(0x5dba5781), V22 = SPH_C32(0x04016ce5), V23 = SPH_C32(0xad659c05), V24 = SPH_C32(0x0306194f), V25 = SPH_C32(0x666d1836), V26 = SPH_C32(0x24aa230a), V27 = SPH_C32(0x8b264ae7);
    sph_u32 V30 = SPH_C32(0x858075d5), V31 = SPH_C32(0x36d79cce), V32 = SPH_C32(0xe571f7d7), V33 = SPH_C32(0x204b1f67), V34 = SPH_C32(0x35870c6a), V35 = SPH_C32(0x57e9e923), V36 = SPH_C32(0x14bcb808), V37 = SPH_C32(0x7cde72ce);
    sph_u32 V40 = SPH_C32(0x6c68e9be), V41 = SPH_C32(0x5ec41e22), V42 = SPH_C32(0xc825b7c7), V43 = SPH_C32(0xaffb4363), V44 = SPH_C32(0xf5df3999), V45 = SPH_C32(0x0fc688f1), V46 = SPH_C32(0xb07224cc), V47 = SPH_C32(0x03e86cea);

    DECL_TMP8(M);

    M0 = SWAP4(hash->h4[0]);
    M1 = SWAP4(hash->h4[1]);
    M2 = SWAP4(hash->h4[2]);
    M3 = SWAP4(hash->h4[3]);
    M4 = SWAP4(hash->h4[4]);
    M5 = SWAP4(hash->h4[5]);
    M6 = SWAP4(hash->h4[6]);
    M7 = SWAP4(hash->h4[7]);

    MI5;
    LUFFA_P5;

    M0 = SPH_C32(0x80000000);
    M1 = 0;
    M2 = 0;
    M3 = 0;
    M4 = 0;
    M5 = 0;
    M6 = 0;
    M7 = 0;

    MI5;
    LUFFA_P5;

    M0 = 0;
    M1 = 0;
    M2 = 0;
    M3 = 0;
    M4 = 0;
    M5 = 0;
    M6 = 0;
    M7 = 0;

    MI5;
    LUFFA_P5;

    hash->h8[0] = ((ulong)SWAP4(V01 ^ V11 ^ V21 ^ V31 ^ V41) << 32) | SWAP4(V00 ^ V10 ^ V20 ^ V30 ^ V40);
    hash->h8[1] = ((ulong)SWAP4(V03 ^ V13 ^ V23 ^ V33 ^ V43) << 32) | SWAP4(V02 ^ V12 ^ V22 ^ V32 ^ V42);
    hash->h8[2] = ((ulong)SWAP4(V05 ^ V15 ^ V25 ^ V35 ^ V45) << 32) | SWAP4(V04 ^ V14 ^ V24 ^ V34 ^ V44);
    hash->h8[3] = ((ulong)SWAP4(V07 ^ V17 ^ V27 ^ V37 ^ V47) << 32) | SWAP4(V06 ^ V16 ^ V26 ^ V36 ^ V46);
}

// cubehash
void hash_x16rs_func_7(hash_32* hash)
{
    sph_u32 x0 = SPH_C32(0x2AEA2A61) ^ hash->h4[0];
    sph_u32 x1 = SPH_C32(0x50F494D4) ^ hash->h4[1];
    sph_u32 x2 = SPH_C32(0x2D538B8B) ^ hash->h4[2];
    sph_u32 x3 = SPH_C32(0x4167D83E) ^ hash->h4[3];
    sph_u32 x4 = SPH_C32(0x3FEE2313) ^ hash->h4[4];
    sph_u32 x5 = SPH_C32(0xC701CF8C) ^ hash->h4[5];
    sph_u32 x6 = SPH_C32(0xCC39968E) ^ hash->h4[6];
    sph_u32 x7 = SPH_C32(0x50AC5695) ^ hash->h4[7];
    sph_u32 x8 = SPH_C32(0x4D42C787), x9 = SPH_C32(0xA647A8B3), xa = SPH_C32(0x97CF0BEF), xb = SPH_C32(0x825B4537);
    sph_u32 xc = SPH_C32(0xEEF864D2), xd = SPH_C32(0xF22090C4), xe = SPH_C32(0xD0E5CD33), xf = SPH_C32(0xA23911AE);
    sph_u32 xg = SPH_C32(0xFCD398D9), xh = SPH_C32(0x148FE485), xi = SPH_C32(0x1B017BEF), xj = SPH_C32(0xB6444532);
    sph_u32 xk = SPH_C32(0x6A536159), xl = SPH_C32(0x2FF5781C), xm = SPH_C32(0x91FA7934), xn = SPH_C32(0x0DBADEA9);
    sph_u32 xo = SPH_C32(0xD65C8A2B), xp = SPH_C32(0xA5A70E75), xq = SPH_C32(0xB1C62456), xr = SPH_C32(0xBC796576);
    sph_u32 xs = SPH_C32(0x1921C8F7), xt = SPH_C32(0xE7989AF1), xu = SPH_C32(0x7795D246), xv = SPH_C32(0xD43E3B44);

    //#pragma unroll 12
    for (int i = 0; i < 12; i ++)
    {
      if(i == 1)
      {
        x0 ^= SPH_C32(0x00000080);
      }
      else if (i == 2)
      {
        xv ^= SPH_C32(1);
      }
      
      SIXTEEN_ROUNDS;
    }

    hash->h8[0] = ((ulong)x1 << 32) | x0;
    hash->h8[1] = ((ulong)x3 << 32) | x2;
    hash->h8[2] = ((ulong)x5 << 32) | x4;
    hash->h8[3] = ((ulong)x7 << 32) | x6;
}

// shavite
void hash_x16rs_func_8(hash_32* hash, __local const sph_u32* AES0, __local const sph_u32* AES1, __local const sph_u32* AES2, __local const sph_u32* AES3)
{
    // IV
    sph_u32 h0 = SPH_C32(0x72FCCDD8), h1 = SPH_C32(0x79CA4727), h2 = SPH_C32(0x128A077B), h3 = SPH_C32(0x40D55AEC);
    sph_u32 h4 = SPH_C32(0xD1901A06), h5 = SPH_C32(0x430AE307), h6 = SPH_C32(0xB29F5CD1), h7 = SPH_C32(0xDF07FBFC);
    sph_u32 h8 = SPH_C32(0x8E45D73D), h9 = SPH_C32(0x681AB538), hA = SPH_C32(0xBDE86578), hB = SPH_C32(0xDD577E47);
    sph_u32 hC = SPH_C32(0xE275EADE), hD = SPH_C32(0x502D9FCD), hE = SPH_C32(0xB9357178), hF = SPH_C32(0x022A4B9A);

    // state
    sph_u32 rk00, rk01, rk02, rk03, rk04, rk05, rk06, rk07;
    sph_u32 rk08, rk09, rk0A, rk0B, rk0C, rk0D, rk0E, rk0F;
    sph_u32 rk10, rk11, rk12, rk13, rk14, rk15, rk16, rk17;
    sph_u32 rk18, rk19, rk1A, rk1B, rk1C, rk1D, rk1E, rk1F;

    sph_u32 sc_count0 = 256;

    rk00 = hash->h4[0];
    rk01 = hash->h4[1];
    rk02 = hash->h4[2];
    rk03 = hash->h4[3];
    rk04 = hash->h4[4];
    rk05 = hash->h4[5];
    rk06 = hash->h4[6];
    rk07 = hash->h4[7];
    rk08 = SPH_C32(0x00000080);
    rk09 = 0;
    rk0A = 0;
    rk0B = 0;
    rk0C = 0;
    rk0D = 0;
    rk0E = 0;
    rk0F = 0;
    rk10 = 0;
    rk11 = 0;
    rk12 = 0;
    rk13 = 0;
    rk14 = 0;
    rk15 = 0;
    rk16 = 0;
    rk17 = 0;
    rk18 = 0;
    rk19 = 0;
    rk1A = 0;
    rk1B = SPH_C32(0x01000000);
    rk1C = 0;
    rk1D = 0;
    rk1E = 0;
    rk1F = SPH_C32(0x02000000);

    c512(buf);

    hash->h8[0] = ((ulong)h1 << 32) | h0;
    hash->h8[1] = ((ulong)h3 << 32) | h2;
    hash->h8[2] = ((ulong)h5 << 32) | h4;
    hash->h8[3] = ((ulong)h7 << 32) | h6;
}

// simd
void hash_x16rs_func_9(hash_32* hash)
{
  s32 ALIGN32 q[256] = { 0 };
  unsigned char ALIGN x[128] = { 0 };
  #pragma unroll 32
  for(unsigned int i = 0; i < 32; i++)
    x[i] = hash->h1[i];

  u32 A0 = C32(0x0BA16B95), A1 = C32(0x72F999AD), A2 = C32(0x9FECC2AE), A3 = C32(0xBA3264FC), A4 = C32(0x5E894929), A5 = C32(0x8E9F30E5), A6 = C32(0x2F1DAA37), A7 = C32(0xF0F2C558);
  u32 B0 = C32(0xAC506643), B1 = C32(0xA90635A5), B2 = C32(0xE25B878B), B3 = C32(0xAAB7878F), B4 = C32(0x88817F7A), B5 = C32(0x0A02892B), B6 = C32(0x559A7550), B7 = C32(0x598F657E);
  u32 C0 = C32(0x7EEF60A1), C1 = C32(0x6B70E3E8), C2 = C32(0x9C1714D1), C3 = C32(0xB958E2A8), C4 = C32(0xAB02675E), C5 = C32(0xED1C014F), C6 = C32(0xCD8D65BB), C7 = C32(0xFDB7A257);
  u32 D0 = C32(0x09254899), D1 = C32(0xD699C7BC), D2 = C32(0x9019B6DC), D3 = C32(0x2B9022E4), D4 = C32(0x8FA14956), D5 = C32(0x21BF9BD3), D6 = C32(0xB94D0943), D7 = C32(0x6FFDDC22);

  FFT256(0, 1, 0, ll1);
  #pragma unroll 256
  for (int i = 0; i < 256; i ++)
  {
    s32 tq;

    tq = q[i] + yoff_b_n[i];
    tq = REDS2(tq);
    tq = REDS1(tq);
    tq = REDS1(tq);
    q[i] = (tq <= 128 ? tq : tq - 257);
  }

  A0 ^= hash->h4[0];
  A1 ^= hash->h4[1];
  A2 ^= hash->h4[2];
  A3 ^= hash->h4[3];
  A4 ^= hash->h4[4];
  A5 ^= hash->h4[5];
  A6 ^= hash->h4[6];
  A7 ^= hash->h4[7];

  ONE_ROUND_BIG(0_, 0,  3, 23, 17, 27);
  ONE_ROUND_BIG(1_, 1, 28, 19, 22,  7);
  ONE_ROUND_BIG(2_, 2, 29,  9, 15,  5);
  ONE_ROUND_BIG(3_, 3,  4, 13, 10, 25);

  STEP_BIG(
    C32(0x0BA16B95), C32(0x72F999AD), C32(0x9FECC2AE), C32(0xBA3264FC),
    C32(0x5E894929), C32(0x8E9F30E5), C32(0x2F1DAA37), C32(0xF0F2C558),
    IF,  4, 13, PP8_4_);

  STEP_BIG(
    C32(0xAC506643), C32(0xA90635A5), C32(0xE25B878B), C32(0xAAB7878F),
    C32(0x88817F7A), C32(0x0A02892B), C32(0x559A7550), C32(0x598F657E),
    IF, 13, 10, PP8_5_);

  STEP_BIG(
    C32(0x7EEF60A1), C32(0x6B70E3E8), C32(0x9C1714D1), C32(0xB958E2A8),
    C32(0xAB02675E), C32(0xED1C014F), C32(0xCD8D65BB), C32(0xFDB7A257),
    IF, 10, 25, PP8_6_);

  STEP_BIG(
    C32(0x09254899), C32(0xD699C7BC), C32(0x9019B6DC), C32(0x2B9022E4),
    C32(0x8FA14956), C32(0x21BF9BD3), C32(0xB94D0943), C32(0x6FFDDC22),
    IF, 25,  4, PP8_0_);

  u32 COPY_A0 = (A0), COPY_A1 = (A1), COPY_A2 = (A2), COPY_A3 = (A3), 
      COPY_A4 = (A4), COPY_A5 = (A5), COPY_A6 = (A6), COPY_A7 = (A7);
  u32 COPY_B0 = (B0), COPY_B1 = (B1), COPY_B2 = (B2), COPY_B3 = (B3), 
      COPY_B4 = (B4), COPY_B5 = (B5), COPY_B6 = (B6), COPY_B7 = (B7);
  u32 COPY_C0 = (C0), COPY_C1 = (C1), COPY_C2 = (C2), COPY_C3 = (C3), 
      COPY_C4 = (C4), COPY_C5 = (C5), COPY_C6 = (C6), COPY_C7 = (C7);
  u32 COPY_D0 = (D0), COPY_D1 = (D1), COPY_D2 = (D2), COPY_D3 = (D3), 
      COPY_D4 = (D4), COPY_D5 = (D5), COPY_D6 = (D6), COPY_D7 = (D7);

  A0 ^= (SPH_C32(0x00000100));

  x[0] = 0;
  x[1] = 1; // FIXED
  #pragma unroll 126
  for(unsigned int i = 2; i < 128; i++)
      x[i] = 0;

  FFT256(0, 1, 0, ll1);
  #pragma unroll 256
  for (int i = 0; i < 256; i ++) {
      s32 tq;

      tq = q[i] + yoff_b_f[i];
      tq = REDS2(tq);
      tq = REDS1(tq);
      tq = REDS1(tq);
      q[i] = (tq <= 128 ? tq : tq - 257);
  }

  ONE_ROUND_BIG(0_, 0,  3, 23, 17, 27);
  ONE_ROUND_BIG(1_, 1, 28, 19, 22,  7);
  ONE_ROUND_BIG(2_, 2, 29,  9, 15,  5);
  ONE_ROUND_BIG(3_, 3,  4, 13, 10, 25);

  STEP_BIG(
    COPY_A0, COPY_A1, COPY_A2, COPY_A3,
    COPY_A4, COPY_A5, COPY_A6, COPY_A7,
    IF,  4, 13, PP8_4_);

  STEP_BIG(
    COPY_B0, COPY_B1, COPY_B2, COPY_B3,
    COPY_B4, COPY_B5, COPY_B6, COPY_B7,
    IF, 13, 10, PP8_5_);

  STEP_BIG(
    COPY_C0, COPY_C1, COPY_C2, COPY_C3,
    COPY_C4, COPY_C5, COPY_C6, COPY_C7,
    IF, 10, 25, PP8_6_);

  STEP_BIG(
    COPY_D0, COPY_D1, COPY_D2, COPY_D3,
    COPY_D4, COPY_D5, COPY_D6, COPY_D7,
    IF, 25,  4, PP8_0_);

  hash->h8[0] = ((ulong)A1 << 32) | A0;
  hash->h8[1] = ((ulong)A3 << 32) | A2;
  hash->h8[2] = ((ulong)A5 << 32) | A4;
  hash->h8[3] = ((ulong)A7 << 32) | A6;
}

// echo
void hash_x16rs_func_10(hash_32* hash, __local const sph_u32* AES0, __local const sph_u32* AES1, __local const sph_u32* AES2, __local const sph_u32* AES3)
{
  sph_u64 W00, W01, W10, W11, W20, W21, W30, W31, W40, W41, W50, W51, W60, W61, W70, W71, W80, W81, W90, W91, WA0, WA1, WB0, WB1, WC0, WC1, WD0, WD1, WE0, WE1, WF0, WF1;

  sph_u32 K0 = SPH_C32(0x00000100);
  sph_u32 K1 = 0;
  sph_u32 K2 = 0;
  sph_u32 K3 = 0;

  W00 = W10 = W20 = W30 = W40 = W50 = W60 = W70 = 512UL;
  W01 = W11 = W21 = W31 = W41 = W51 = W61 = W71 = WA1 = WB0 = WB1 = WC0 = WC1 = WD0 = WD1 = WE0 = WF1 = 0;

  W80 = hash->h8[0];
  W81 = hash->h8[1];
  W90 = hash->h8[2];
  W91 = hash->h8[3];
  WA0 = SPH_C64(0x0000000000000080);
  WE1 = SPH_C64(0x0200000000000000);
  WF0 = SPH_C64(0x0000000000000100);

  //#pragma unroll 10
  for (unsigned u = 0; u < 9; u ++)
    BIG_ROUND;

  BIG_ROUND_LAST;

  hash->h8[0] = hash->h8[0] ^ 512UL ^ W00 ^ W80;
  hash->h8[1] = hash->h8[1] ^ 0 ^ W01 ^ W81;
  hash->h8[2] = hash->h8[2] ^ 512UL ^ W10 ^ W90;
  hash->h8[3] = hash->h8[3] ^ 0 ^ W11 ^ W91;
}

// hamsi
void hash_x16rs_func_11(hash_32* hash32)
{
  hash_t ALIGN hash = { 0 };
  for(int i = 0; i < 4; i++) {
      hash.h8[i] = hash32->h8[i];
  }
  hash.h8[5] = 4113;
  hash.h8[6] = 5278886529044860489;
  hash.h8[7] = 4188411865185001543;
              /*if(get_global_id(0)==0) {
                printf("hash[5]: %llu\n", hash.h8[5]);
                printf("hash[6]: %llu\n", hash.h8[6]);
                printf("hash[7]: %llu\n", hash.h8[7]);
              }*/

  sph_u32 ALIGN32 T512_L[1024];

  //#pragma unroll 1024
  for (int i = 0; i < 1024; i++)
    T512_L[i] = T512_W[i];

  sph_u32 c0 = HAMSI_IV512[0], c1 = HAMSI_IV512[1], c2 = HAMSI_IV512[2], c3 = HAMSI_IV512[3];
  sph_u32 c4 = HAMSI_IV512[4], c5 = HAMSI_IV512[5], c6 = HAMSI_IV512[6], c7 = HAMSI_IV512[7];
  sph_u32 c8 = HAMSI_IV512[8], c9 = HAMSI_IV512[9], cA = HAMSI_IV512[10], cB = HAMSI_IV512[11];
  sph_u32 cC = HAMSI_IV512[12], cD = HAMSI_IV512[13], cE = HAMSI_IV512[14], cF = HAMSI_IV512[15];
  sph_u32 m0, m1, m2, m3, m4, m5, m6, m7;
  sph_u32 m8, m9, mA, mB, mC, mD, mE, mF;
  sph_u32 ALIGN32 h[16] = { c0, c1, c2, c3, c4, c5, c6, c7, c8, c9, cA, cB, cC, cD, cE, cF };

  #define buf(u) hash.h1[i + u]

  for(int i = 0; i < 32; i += 8)
  {
    INPUT_BIG;
    P_BIG;
    T_BIG;
  }

  #undef buf
  #define buf(u) (u == 0 ? 0x80 : 0)

  INPUT_BIG;
  P_BIG;
  T_BIG;

  #undef buf
  #define buf(u) (u == 6 ? 1 : 0)

  INPUT_BIG;
  PF_BIG;
  T_BIG_LAST;

  hash32->h8[0] = ((ulong)SWAP4(h[1]) << 32) | SWAP4(h[0]);
  hash32->h8[1] = ((ulong)SWAP4(h[3]) << 32) | SWAP4(h[2]);
  hash32->h8[2] = ((ulong)SWAP4(h[5]) << 32) | SWAP4(h[4]);
  hash32->h8[3] = ((ulong)SWAP4(h[7]) << 32) | SWAP4(h[6]);
}

// fugue
void hash_x16rs_func_12(hash_32* hash, sph_u32* mixtab0, sph_u32* mixtab1, sph_u32* mixtab2, sph_u32* mixtab3)
{  
  sph_u32 S00 = 0;
  sph_u32 S01 = 0;
  sph_u32 S02 = 0;
  sph_u32 S03 = 0;
  sph_u32 S04 = 0;
  sph_u32 S05 = 0;
  sph_u32 S06 = 0;
  sph_u32 S07 = 0;
  sph_u32 S08 = 0;
  sph_u32 S09 = 0;
  sph_u32 S10 = 0;
  sph_u32 S11 = 0;
  sph_u32 S12 = 0;
  sph_u32 S13 = 0;
  sph_u32 S14 = 0;
  sph_u32 S15 = 0;
  sph_u32 S16 = 0;
  sph_u32 S17 = 0;
  sph_u32 S18 = 0;
  sph_u32 S19 = 0;
  sph_u32 S20 = SPH_C32(0x8807a57e);
  sph_u32 S21 = SPH_C32(0xe616af75);
  sph_u32 S22 = SPH_C32(0xc5d3e4db);
  sph_u32 S23 = SPH_C32(0xac9ab027);
  sph_u32 S24 = SPH_C32(0xd915f117);
  sph_u32 S25 = SPH_C32(0xb6eecc54);
  sph_u32 S26 = SPH_C32(0x06e8020b);
  sph_u32 S27 = SPH_C32(0x4a92efd1);
  sph_u32 S28 = SPH_C32(0xaac6e2c9);
  sph_u32 S29 = SPH_C32(0xddb21398);
  sph_u32 S30 = SPH_C32(0xcae65838);
  sph_u32 S31 = SPH_C32(0x437f203f);
  sph_u32 S32 = SPH_C32(0x25ea78e7);
  sph_u32 S33 = SPH_C32(0x951fddd6);
  sph_u32 S34 = SPH_C32(0xda6ed11d);
  sph_u32 S35 = SPH_C32(0xe13e3567);

  FUGUE512_one(SWAP4(hash->h4[0]));
  FUGUE512_two(SWAP4(hash->h4[1]));
  FUGUE512_trd(SWAP4(hash->h4[2]));
  FUGUE512_one(SWAP4(hash->h4[3]));
  FUGUE512_two(SWAP4(hash->h4[4]));
  FUGUE512_trd(SWAP4(hash->h4[5]));
  FUGUE512_one(SWAP4(hash->h4[6]));
  FUGUE512_two(SWAP4(hash->h4[7]));
  FUGUE512_trd((0));
  FUGUE512_one((SPH_C32(0x00000100)));

  ROR12;

  // apply round shift if necessary
  int i;

  for (i = 0; i < 32; i ++)
  {
    ROR3;
    CMIX36(S00, S01, S02, S04, S05, S06, S18, S19, S20);
    SMIX(S00, S01, S02, S03);
  }

  for (i = 0; i < 13; i ++)
  {
    S04 ^= S00;
    S09 ^= S00;
    S18 ^= S00;
    S27 ^= S00;
    ROR9;
    SMIX(S00, S01, S02, S03);
    S04 ^= S00;
    S10 ^= S00;
    S18 ^= S00;
    S27 ^= S00;
    ROR9;
    SMIX(S00, S01, S02, S03);
    S04 ^= S00;
    S10 ^= S00;
    S19 ^= S00;
    S27 ^= S00;
    ROR9;
    SMIX(S00, S01, S02, S03);
    S04 ^= S00;
    S10 ^= S00;
    S19 ^= S00;
    S28 ^= S00;
    ROR8;
    SMIX(S00, S01, S02, S03);
  }

  S04 ^= S00;
  S09 ^= S00;
  hash->h8[0] = ((ulong)ENC32E(S02) << 32) | ENC32E(S01);
  hash->h8[1] = ((ulong)ENC32E(S04) << 32) | ENC32E(S03);
  hash->h8[2] = ((ulong)ENC32E(S10) << 32) | ENC32E(S09);
  hash->h8[3] = ((ulong)ENC32E(S12) << 32) | ENC32E(S11);
}

// shabal
void hash_x16rs_func_13(hash_32* hash)
{
  sph_u32 A00 = A_init_512[0], A01 = A_init_512[1], A02 = A_init_512[2], A03 = A_init_512[3], A04 = A_init_512[4], A05 = A_init_512[5], A06 = A_init_512[6], A07 = A_init_512[7],
  A08 = A_init_512[8], A09 = A_init_512[9], A0A = A_init_512[10], A0B = A_init_512[11];
  sph_u32 B0 = B_init_512[0], B1 = B_init_512[1], B2 = B_init_512[2], B3 = B_init_512[3], B4 = B_init_512[4], B5 = B_init_512[5], B6 = B_init_512[6], B7 = B_init_512[7],
  B8 = B_init_512[8], B9 = B_init_512[9], BA = B_init_512[10], BB = B_init_512[11], BC = B_init_512[12], BD = B_init_512[13], BE = B_init_512[14], BF = B_init_512[15];
  sph_u32 C0 = C_init_512[0], C1 = C_init_512[1], C2 = C_init_512[2], C3 = C_init_512[3], C4 = C_init_512[4], C5 = C_init_512[5], C6 = C_init_512[6], C7 = C_init_512[7],
  C8 = C_init_512[8], C9 = C_init_512[9], CA = C_init_512[10], CB = C_init_512[11], CC = C_init_512[12], CD = C_init_512[13], CE = C_init_512[14], CF = C_init_512[15];
  sph_u32 M0 = hash->h4[0], M1 = hash->h4[1], M2 = hash->h4[2], M3 = hash->h4[3], M4 = hash->h4[4], M5 = hash->h4[5], M6 = hash->h4[6], M7 = hash->h4[7], M8 = SPH_C32(0x00000080), M9 = 0, MA = 0, MB = 0, MC = 0, MD = 0, ME = 0, MF = 0;
  sph_u32 Wlow = 1, Whigh = 0;

  INPUT_BLOCK_ADD;
  XOR_W;
  APPLY_P;

  SWAP_BC;
  XOR_W;
  APPLY_P;

  SWAP_BC;
  XOR_W;
  APPLY_P;

  SWAP_BC;
  XOR_W;
  APPLY_P_LAST;

  hash->h8[0] = ((ulong)B1 << 32) | B0;
  hash->h8[1] = ((ulong)B3 << 32) | B2;
  hash->h8[2] = ((ulong)B5 << 32) | B4;
  hash->h8[3] = ((ulong)B7 << 32) | B6;
}

// whirlpool
void hash_x16rs_func_14(hash_32* hash, __local const sph_u64* ALIGN LT0, __local const sph_u64* ALIGN LT1, __local const sph_u64* ALIGN LT2, __local const sph_u64* ALIGN LT3, __local const sph_u64* ALIGN LT4, __local const sph_u64* ALIGN LT5, __local const sph_u64* ALIGN LT6, __local const sph_u64* ALIGN LT7)
{
    sph_u64 n0 = hash->h8[0];
    sph_u64 n1 = hash->h8[1];
    sph_u64 n2 = hash->h8[2];
    sph_u64 n3 = hash->h8[3];
    sph_u64 n4 = SPH_C64(0x0000000000000080);
    sph_u64 n5 = 0;
    sph_u64 n6 = 0;
    sph_u64 n7 = 0;
    sph_u64 h0 = 0;
    sph_u64 h1 = 0;
    sph_u64 h2 = 0;
    sph_u64 h3 = 0;
    sph_u64 h4 = 0;
    sph_u64 h5 = 0;
    sph_u64 h6 = 0;
    sph_u64 h7 = 0;

    //#pragma unroll 10
    for (unsigned r = 0; r < 10; r ++)
    {
        sph_u64 ALIGN32 tmp[8];

        ROUND_KSCHED(LT, h, tmp, plain_RC[r]);
        TRANSFER(h, tmp);
        ROUND_WENC(LT, n, h, tmp);
        TRANSFER(n, tmp);
    }

    const sph_u64 state[4] = { n0 ^ hash->h8[0], n1 ^ hash->h8[1], n2 ^ hash->h8[2], n3 ^ hash->h8[3] };

    h0 = state[0];
    h1 = state[1];
    h2 = state[2];
    h3 = state[3];
    h4 = n4 ^ SPH_C64(0x0000000000000080);
    h5 = n5;
    h6 = n6;
    h7 = n7;

    n0 = h0;
    n1 = h1;
    n2 = h2;
    n3 = h3;
    n4 = h4;
    n7 = SPH_C64(0x0001000000000000) ^ h7;

    //#pragma unroll 10
    for (unsigned r = 0; r < 10; r ++)
    {
        sph_u64 ALIGN32 tmp[8];

        ROUND_KSCHED(LT, h, tmp, plain_RC[r]);
        TRANSFER(h, tmp);
        if(r == 9) {
          ROUND_WENC_LAST(LT, n, h, tmp);
          TRANSFER_LAST(n, tmp);
        } else {
          ROUND_WENC(LT, n, h, tmp);
          TRANSFER(n, tmp);
        }
    }

    hash->h8[0] = (state[0] ^ n0 ^ hash->h8[0]) ^ hash->h8[0];
    hash->h8[1] = (state[1] ^ n1 ^ hash->h8[1]) ^ hash->h8[1];
    hash->h8[2] = (state[2] ^ n2 ^ hash->h8[2]) ^ hash->h8[2];
    hash->h8[3] = (state[3] ^ n3 ^ hash->h8[3]) ^ hash->h8[3];
}

// sha2
void hash_x16rs_func_15(hash_32* hash)
{
    unsigned char ALIGN digest[32];
    easy_sha512(hash->h1, digest);
    uint64_t* digest64 = (uint64_t*)digest;
    hash->h8[0] = ((uint64_t)digest[0])    |
              ((uint64_t)digest[1] << 8)   |
              ((uint64_t)digest[2] << 16)  |
              ((uint64_t)digest[3] << 24)  |
              ((uint64_t)digest[4] << 32)  |
              ((uint64_t)digest[5] << 40)  |
              ((uint64_t)digest[6] << 48)  |
              ((uint64_t)digest[7] << 56);
    hash->h8[1] = ((uint64_t)digest[8])        |
                  ((uint64_t)digest[9] << 8)   |
                  ((uint64_t)digest[10] << 16) |
                  ((uint64_t)digest[11] << 24) |
                  ((uint64_t)digest[12] << 32) |
                  ((uint64_t)digest[13] << 40) |
                  ((uint64_t)digest[14] << 48) |
                  ((uint64_t)digest[15] << 56);
    hash->h8[2] = ((uint64_t)digest[16])       |
                  ((uint64_t)digest[17] << 8)  |
                  ((uint64_t)digest[18] << 16) |
                  ((uint64_t)digest[19] << 24) |
                  ((uint64_t)digest[20] << 32) |
                  ((uint64_t)digest[21] << 40) |
                  ((uint64_t)digest[22] << 48) |
                  ((uint64_t)digest[23] << 56);
    hash->h8[3] = ((uint64_t)digest[24])       |
                  ((uint64_t)digest[25] << 8)  |
                  ((uint64_t)digest[26] << 16) |
                  ((uint64_t)digest[27] << 24) |
                  ((uint64_t)digest[28] << 32) |
                  ((uint64_t)digest[29] << 40) |
                  ((uint64_t)digest[30] << 48) |
                  ((uint64_t)digest[31] << 56);
}

#endif // X16RX_CL