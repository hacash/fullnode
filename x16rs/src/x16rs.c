#include "x16rs.h"
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <stdio.h>

#pragma GCC diagnostic ignored "-Wpointer-sign"

// void print_byte_list(char* name, void *data, int len, int wide);
/*
#include "sha3/sph_blake.h"
#include "sha3/sph_bmw.h"
#include "sha3/sph_groestl.h"
#include "sha3/sph_jh.h"
#include "sha3/sph_keccak.h"
#include "sha3/sph_skein.h"
#include "sha3/sph_luffa.h"
#include "sha3/sph_cubehash.h"
#include "sha3/sph_shavite.h"
#include "sha3/sph_simd.h"
#include "sha3/sph_echo.h"
#include "sha3/sph_hamsi.h"
#include "sha3/sph_fugue.h"
#include "sha3/sph_shabal.h"
#include "sha3/sph_whirlpool.h"
#include "sha3/sph_sha2.h"
*/
// #include "sha3_256/sha3.h"


#include "sha3/sha2big.c"
#include "sha3/aes_helper.c"
#include "sha3/blake.c"
#include "sha3/bmw.c"
#include "sha3/groestl.c"
#include "sha3/jh.c"
#include "sha3/keccak.c"
#include "sha3/skein.c"
#include "sha3/luffa.c"
#include "sha3/cubehash.c"
#include "sha3/shavite.c"
#include "sha3/simd.c"
#include "sha3/echo.c"
#include "sha3/hamsi.c"
#include "sha3/fugue.c"
#include "sha3/shabal.c"
#include "sha3/whirlpool.c"



enum Algo {
    BLAKE = 0,
    BMW,
    GROESTL,
    JH,
    KECCAK,
    SKEIN,
    LUFFA,
    CUBEHASH,
    SHAVITE,
    SIMD,
    ECHO,
    HAMSI,
    FUGUE,
    SHABAL,
    WHIRLPOOL,
    SHA512
};

// input length must more than 32
static const size_t x16rs_hash_insize = 32;
void c_x16rs_hash(const int loopnum, const char* input_hash, char* output_hash)
{
    // printf("\n"); fflush(stdout);
    uint32_t inputoutput[64/4];

    uint32_t *input_hash_ptr32 = (uint32_t *) input_hash;
    inputoutput[0] = input_hash_ptr32[0];
    inputoutput[1] = input_hash_ptr32[1];
    inputoutput[2] = input_hash_ptr32[2];
    inputoutput[3] = input_hash_ptr32[3];
    inputoutput[4] = input_hash_ptr32[4];
    inputoutput[5] = input_hash_ptr32[5];
    inputoutput[6] = input_hash_ptr32[6];
    inputoutput[7] = input_hash_ptr32[7];

    sph_blake512_context     ctx_blake;
    sph_bmw512_context       ctx_bmw;
    sph_groestl512_context   ctx_groestl;
    sph_skein512_context     ctx_skein;
    sph_jh512_context        ctx_jh;
    sph_keccak512_context    ctx_keccak;
    sph_luffa512_context     ctx_luffa;
    sph_cubehash512_context  ctx_cubehash;
    sph_shavite512_context   ctx_shavite;
    sph_simd512_context      ctx_simd;
    sph_echo512_context      ctx_echo;
    sph_hamsi512_context     ctx_hamsi;
    sph_fugue512_context     ctx_fugue;
    sph_shabal512_context    ctx_shabal;
    sph_whirlpool_context    ctx_whirlpool;
    sph_sha512_context       ctx_sha512;

    int n;
    for(n = 0; n < loopnum; n++){

        uint8_t algo = inputoutput[7] % 16;
        switch (algo)
        {
        case BLAKE:
            // printf("BLAKE,");
            sph_blake512_init(&ctx_blake);
            sph_blake512(&ctx_blake, inputoutput, x16rs_hash_insize);
            sph_blake512_close(&ctx_blake, inputoutput);
        break;
        case BMW:
            // printf("BMW,");
            sph_bmw512_init(&ctx_bmw);
            sph_bmw512(&ctx_bmw, inputoutput, x16rs_hash_insize);
            sph_bmw512_close(&ctx_bmw, inputoutput);
        break;
        case GROESTL:
            // printf("GROESTL,");
            sph_groestl512_init(&ctx_groestl);
            sph_groestl512(&ctx_groestl, inputoutput, x16rs_hash_insize);
            sph_groestl512_close(&ctx_groestl, inputoutput);
        break;
        case SKEIN:
            // printf("SKEIN,");
            sph_skein512_init(&ctx_skein);
            sph_skein512(&ctx_skein, inputoutput, x16rs_hash_insize);
            sph_skein512_close(&ctx_skein, inputoutput);
        break;
        case JH:
            // printf("JH,");
            sph_jh512_init(&ctx_jh);
            sph_jh512(&ctx_jh, inputoutput, x16rs_hash_insize);
            sph_jh512_close(&ctx_jh, inputoutput);
        break;
        case KECCAK:
            // printf("KECCAK,");
            sph_keccak512_init(&ctx_keccak);
            sph_keccak512(&ctx_keccak, inputoutput, x16rs_hash_insize);
            sph_keccak512_close(&ctx_keccak, inputoutput);
        break;
        case LUFFA:
            // printf("LUFFA,");
            sph_luffa512_init(&ctx_luffa);
            sph_luffa512(&ctx_luffa, inputoutput, x16rs_hash_insize);
            sph_luffa512_close(&ctx_luffa, inputoutput);
        break;
        case CUBEHASH:
            // printf("CUBEHASH,");
            sph_cubehash512_init(&ctx_cubehash);
            sph_cubehash512(&ctx_cubehash, inputoutput, x16rs_hash_insize);
            sph_cubehash512_close(&ctx_cubehash, inputoutput);
        break;
        case SHAVITE:
            // printf("SHAVITE,");
            sph_shavite512_init(&ctx_shavite);
            sph_shavite512(&ctx_shavite, inputoutput, x16rs_hash_insize);
            sph_shavite512_close(&ctx_shavite, inputoutput);
        break;
        case SIMD:
            // printf("SIMD,");
            sph_simd512_init(&ctx_simd);
            sph_simd512(&ctx_simd, inputoutput, x16rs_hash_insize);
            sph_simd512_close(&ctx_simd, inputoutput);
        break;
        case ECHO:
            // printf("ECHO,");
            sph_echo512_init(&ctx_echo);
            sph_echo512(&ctx_echo, inputoutput, x16rs_hash_insize);
            sph_echo512_close(&ctx_echo, inputoutput);
        break;
        case HAMSI:
            // printf("HAMSI,");
            sph_hamsi512_init(&ctx_hamsi);
            sph_hamsi512(&ctx_hamsi, inputoutput, x16rs_hash_insize);
            sph_hamsi512_close(&ctx_hamsi, inputoutput);
        break;
        case FUGUE:
            // printf("FUGUE,");
            sph_fugue512_init(&ctx_fugue);
            sph_fugue512(&ctx_fugue, inputoutput, x16rs_hash_insize);
            sph_fugue512_close(&ctx_fugue, inputoutput);
        break;
        case SHABAL:
            // printf("SHABAL,");
            sph_shabal512_init(&ctx_shabal);
            sph_shabal512(&ctx_shabal, inputoutput, x16rs_hash_insize);
            sph_shabal512_close(&ctx_shabal, inputoutput);
        break;
        case WHIRLPOOL:
            // printf("WHIRLPOOL,");
            sph_whirlpool_init(&ctx_whirlpool);
            sph_whirlpool(&ctx_whirlpool, inputoutput, x16rs_hash_insize);
            sph_whirlpool_close(&ctx_whirlpool, inputoutput);
        break;
        case SHA512:
            // printf("SHA512,");
            sph_sha512_init(&ctx_sha512);
            sph_sha512(&ctx_sha512, inputoutput, x16rs_hash_insize);
            sph_sha512_close(&ctx_sha512, inputoutput);
        break;
        }

    }
            // printf("output_hash_ptr32");

    uint32_t *output_hash_ptr32 = (uint32_t *) output_hash;
    output_hash_ptr32[0] = inputoutput[0];
    output_hash_ptr32[1] = inputoutput[1];
    output_hash_ptr32[2] = inputoutput[2];
    output_hash_ptr32[3] = inputoutput[3];
    output_hash_ptr32[4] = inputoutput[4];
    output_hash_ptr32[5] = inputoutput[5];
    output_hash_ptr32[6] = inputoutput[6];
    output_hash_ptr32[7] = inputoutput[7];
}


// void main() {  }