#ifndef SHA512_CL
#define SHA512_CL

/*
 * Copyright (c) 2018, Jiamin Ma
 * Modified for Hacash x16rs by Ivan Martin
 * BSD License
 */
#ifndef UINT64_T_DEFINED
#define UINT64_T_DEFINED
typedef unsigned long uint64_t;
#endif
typedef unsigned int uint32_t;
typedef unsigned char uint8_t;

#define SHA512_DEBUG(fmt) ()

/**
 * @brief   Convert uint64_t to big endian byte array.
 * @param   input       input uint64_t data
 * @param   output      output big endian byte array
 * @param   idx         idx of the byte array.
 * @retval  void
 */
static void inline sha512_encode(uint64_t input, uint8_t output[], uint32_t idx)
{
    output[idx + 0] = (uint8_t)(input >> 56);
    output[idx + 1] = (uint8_t)(input >> 48);
    output[idx + 2] = (uint8_t)(input >> 40);
    output[idx + 3] = (uint8_t)(input >> 32);
    output[idx + 4] = (uint8_t)(input >> 24);
    output[idx + 5] = (uint8_t)(input >> 16);
    output[idx + 6] = (uint8_t)(input >>  8);
    output[idx + 7] = (uint8_t)(input >>  0);
}

/**
 * @brief   Convert big endian byte array to uint64_t data
 * @param   output      output uint64_t data
 * @param   input       input big endian byte array
 * @param   idx         idx of the byte array.
 * @retval  void
 */
static inline void sha512_decode(uint64_t *output, __generic uint8_t input[], uint32_t idx)
{
    *output = ((uint64_t)input[idx + 0] << 56)
            | ((uint64_t)input[idx + 1] << 48)
            | ((uint64_t)input[idx + 2] << 40)
            | ((uint64_t)input[idx + 3] << 32)
            | ((uint64_t)input[idx + 4] << 24)
            | ((uint64_t)input[idx + 5] << 16)
            | ((uint64_t)input[idx + 6] <<  8)
            | ((uint64_t)input[idx + 7] <<  0);
}

typedef struct sha512_ctx_tag {

    uint8_t block[128];
    /*SHA512 will fill 128 bits length field: unit:bit*/
    uint64_t len[2];
    /*Hash values*/
    uint64_t val[8];
    /*Payload address to hash*/
    uint8_t *payload_addr;
} sha512_ctx_t;


#define I64(x) x##UL
#define LSR(x,n) (x >> n)
#define ROR(x,n) (LSR(x,n) | (x << (64 - n)))

#define MA(x,y,z) ((x & y) | (z & (x | y)))
#define CH(x,y,z) (z ^ (x & (y ^ z)))
#define GAMMA0(x) (ROR(x, 1) ^ ROR(x, 8) ^  LSR(x, 7))
#define GAMMA1(x) (ROR(x,19) ^ ROR(x,61) ^  LSR(x, 6))
#define SIGMA0(x) (ROR(x,28) ^ ROR(x,34) ^ ROR(x,39))
#define SIGMA1(x) (ROR(x,14) ^ ROR(x,18) ^ ROR(x,41))

#define INIT_COMPRESSOR() uint64_t tmp0 = 0, tmp1 = 0
#define COMPRESS( a,  b,  c, d,  e,  f,  g,  h, x,  k)   \
    tmp0 = h + SIGMA1(e) + CH(e,f,g) + k + x;              \
    tmp1 = SIGMA0(a) + MA(a,b,c); d += tmp0; h = tmp0 + tmp1;

/*
 * Predefined sha512 padding bytes
 */
__constant static const uint8_t sha512_padding[80] =
{
    0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
};

/*
 * K byte array used for iteration
 */
__constant static const uint64_t K[80] =
{
    I64(0x428A2F98D728AE22),  I64(0x7137449123EF65CD), I64(0xB5C0FBCFEC4D3B2F),  I64(0xE9B5DBA58189DBBC),
    I64(0x3956C25BF348B538),  I64(0x59F111F1B605D019), I64(0x923F82A4AF194F9B),  I64(0xAB1C5ED5DA6D8118),
    I64(0xD807AA98A3030242),  I64(0x12835B0145706FBE), I64(0x243185BE4EE4B28C),  I64(0x550C7DC3D5FFB4E2),
    I64(0x72BE5D74F27B896F),  I64(0x80DEB1FE3B1696B1), I64(0x9BDC06A725C71235),  I64(0xC19BF174CF692694),
    I64(0xE49B69C19EF14AD2),  I64(0xEFBE4786384F25E3), I64(0x0FC19DC68B8CD5B5),  I64(0x240CA1CC77AC9C65),
    I64(0x2DE92C6F592B0275),  I64(0x4A7484AA6EA6E483), I64(0x5CB0A9DCBD41FBD4),  I64(0x76F988DA831153B5),
    I64(0x983E5152EE66DFAB),  I64(0xA831C66D2DB43210), I64(0xB00327C898FB213F),  I64(0xBF597FC7BEEF0EE4),
    I64(0xC6E00BF33DA88FC2),  I64(0xD5A79147930AA725), I64(0x06CA6351E003826F),  I64(0x142929670A0E6E70),
    I64(0x27B70A8546D22FFC),  I64(0x2E1B21385C26C926), I64(0x4D2C6DFC5AC42AED),  I64(0x53380D139D95B3DF),
    I64(0x650A73548BAF63DE),  I64(0x766A0ABB3C77B2A8), I64(0x81C2C92E47EDAEE6),  I64(0x92722C851482353B),
    I64(0xA2BFE8A14CF10364),  I64(0xA81A664BBC423001), I64(0xC24B8B70D0F89791),  I64(0xC76C51A30654BE30),
    I64(0xD192E819D6EF5218),  I64(0xD69906245565A910), I64(0xF40E35855771202A),  I64(0x106AA07032BBD1B8),
    I64(0x19A4C116B8D2D0C8),  I64(0x1E376C085141AB53), I64(0x2748774CDF8EEB99),  I64(0x34B0BCB5E19B48A8),
    I64(0x391C0CB3C5C95A63),  I64(0x4ED8AA4AE3418ACB), I64(0x5B9CCA4F7763E373),  I64(0x682E6FF3D6B2B8A3),
    I64(0x748F82EE5DEFB2FC),  I64(0x78A5636F43172F60), I64(0x84C87814A1F0AB72),  I64(0x8CC702081A6439EC),
    I64(0x90BEFFFA23631E28),  I64(0xA4506CEBDE82BDE9), I64(0xBEF9A3F7B2C67915),  I64(0xC67178F2E372532B),
    I64(0xCA273ECEEA26619C),  I64(0xD186B8C721C0C207), I64(0xEADA7DD6CDE0EB1E),  I64(0xF57D4F7FEE6ED178),
    I64(0x06F067AA72176FBA),  I64(0x0A637DC5A2C898A6), I64(0x113F9804BEF90DAE),  I64(0x1B710B35131C471B),
    I64(0x28DB77F523047D84),  I64(0x32CAAB7B40C72493), I64(0x3C9EBE0A15C9BEBC),  I64(0x431D67C49C100D4C),
    I64(0x4CC5D4BECB3E42B6),  I64(0x597F299CFC657E2A), I64(0x5FCB6FAB3AD6FAEC),  I64(0x6C44198C4A475817)
};



static inline void sha512_memcpy_sha512_padding(uint8_t *dst, uint32_t size)
{
    uint32_t i = 0;
    for (;i < size;i++) {
        *dst++ = sha512_padding[i];
    }
}

static inline void sha512_memcpy(uint8_t *src, uint8_t *dst, uint32_t size)
{
    uint32_t i = 0;
    for (;i < size;i++) {
        *dst++ = *src++;
    }
}

/**
 * @brief   Init the SHA384/SHA512 Context
 * @param   sha512_ctx      SHA384/512 context
 * @param   payload         address of the hash payload
 */
static inline void sha512_init(sha512_ctx_t *sha512_ctx, uint8_t *payload_addr)
{
    sha512_ctx->val[0] = I64(0x6A09E667F3BCC908);
    sha512_ctx->val[1] = I64(0xBB67AE8584CAA73B);
    sha512_ctx->val[2] = I64(0x3C6EF372FE94F82B);
    sha512_ctx->val[3] = I64(0xA54FF53A5F1D36F1);
    sha512_ctx->val[4] = I64(0x510E527FADE682D1);
    sha512_ctx->val[5] = I64(0x9B05688C2B3E6C1F);
    sha512_ctx->val[6] = I64(0x1F83D9ABFB41BD6B);
    sha512_ctx->val[7] = I64(0x5BE0CD19137E2179);

    sha512_ctx->payload_addr = payload_addr;
    sha512_ctx->len[0] = 32 << 3;
    sha512_ctx->len[1] = 32 >> 61;    
}

/**
 * @brief   SHA384/512 iteration compression
 * @param   sha512_ctx        context of the sha384/512
 * @param   data              hash block data, 1024 bits.
 */
static void sha512_hash_factory(sha512_ctx_t *ctx, __generic uint8_t ALIGN data[128])
{
    uint32_t i = 0;
    uint64_t W[80];
    /* One iteration vectors
     * v[0] --> A
     * ...
     * v[7] --> H
     * */
    uint64_t v[8];

    INIT_COMPRESSOR();
    // SHA512_DEBUG("%s\n", __func__);

    /* 1. Calculate the W[80] */
    for(i = 0; i < 16; i++) {
        sha512_decode(&W[i], data, i << 3 );
    }

    for(; i < 80; i++) {
        W[i] = GAMMA1(W[i -  2]) + W[i -  7] + GAMMA0(W[i - 15]) + W[i - 16];
    }

    /* 2.Init the vectors */
    for (i = 0;i < 8; i++) {
        v[i] = ctx->val[i];
    }

    /* 3. Iteration to do the SHA-2 family compression. */
    for(i = 0; i < 80;) {
        COMPRESS(v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], W[i], K[i] ); i++;
        COMPRESS(v[7], v[0], v[1], v[2], v[3], v[4], v[5], v[6], W[i], K[i] ); i++;
        COMPRESS(v[6], v[7], v[0], v[1], v[2], v[3], v[4], v[5], W[i], K[i] ); i++;
        COMPRESS(v[5], v[6], v[7], v[0], v[1], v[2], v[3], v[4], W[i], K[i] ); i++;
        COMPRESS(v[4], v[5], v[6], v[7], v[0], v[1], v[2], v[3], W[i], K[i] ); i++;
        COMPRESS(v[3], v[4], v[5], v[6], v[7], v[0], v[1], v[2], W[i], K[i] ); i++;
        COMPRESS(v[2], v[3], v[4], v[5], v[6], v[7], v[0], v[1], W[i], K[i] ); i++;
        COMPRESS(v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[0], W[i], K[i] ); i++;

    }

    /* 4. Move the vectors to hash output */
    for (i = 0; i < 8; i++) {
        ctx->val[i] += v[i];
    }
}

/**
 * @brief   SHA384/512 stage2:Do padding and digest the fianl bytes
 * @param   sha512_ctx        context of the sha384/512
 * @param   output            output of hash value
 */
static void sha512_stage2(sha512_ctx_t *sha512_ctx,
        uint8_t output[32])
{

    uint8_t temp_data[128] = {0};
    uint8_t *temp_data_p = (uint8_t *)&temp_data[0];
    uint8_t len_be[16] = {0};

    // SHA512_DEBUG("%s\n", __func__);

    /*Copy the last byte to the temp buffer*/
    sha512_memcpy(sha512_ctx->payload_addr, temp_data_p, 32);
    temp_data_p += 32;

    /*Copy the padding byte to the temp buffer*/
    sha512_memcpy_sha512_padding(temp_data_p, 80);
    temp_data_p += 80;

    /*Append the length*/
    sha512_encode(sha512_ctx->len[1], len_be, 0);
    sha512_encode(sha512_ctx->len[0], len_be, 8);
    sha512_memcpy(len_be, temp_data_p, 16);
    sha512_hash_factory(sha512_ctx, temp_data);

    /*encode the hash val to big endian byte array*/
    for (unsigned i = 0; i < 6; i++) {
        sha512_encode(sha512_ctx->val[i], output, i * 8);
    }
}

/**
 * @brief   SHA384/512 implementation function
 * @param   payload         address of the hash payload
 * @param   hash            output of hash value
 */
void easy_sha512_impl(uint8_t *payload,
        uint8_t output[32])
{
    sha512_ctx_t g_sha512_ctx;
    sha512_init(&g_sha512_ctx, payload);
    sha512_stage2(&g_sha512_ctx, output);
}

/**
 * @brief   API for SHA512
 * @param   payload         address of the hash payload
 * @param   hash            output of hash value
 */
void easy_sha512(uint8_t *payload, uint8_t ALIGN hash[32])
{
    return easy_sha512_impl(payload, hash);
}





#endif  // SHA512_CL

