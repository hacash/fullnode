#ifndef X16RX_UTIL_CL
#define X16RX_UTIL_CL

#define ALIGN8 __attribute__((aligned(8)))
#define ALIGN __attribute__((aligned(16)))
#define ALIGN32 __attribute__((aligned(32)))
#define ALIGN64 __attribute__((aligned(64)))

typedef union ALIGN8 {
  unsigned char h1[88];
  ulong h8[11];
} block_t;

#ifdef __ENDIAN_LITTLE__

    #define WRITE_NONCE_BYTE4 bytes[offset+0] = nonce_ptr[3]; \
    bytes[offset+1] = nonce_ptr[2];\
    bytes[offset+2] = nonce_ptr[1];\
    bytes[offset+3] = nonce_ptr[0];

#else

    #define WRITE_NONCE_BYTE4 bytes[offset+0] = nonce_ptr[0];\
    bytes[offset+1] = nonce_ptr[1];\
    bytes[offset+2] = nonce_ptr[2];\
    bytes[offset+3] = nonce_ptr[3];

#endif

__inline__ void write_nonce_to_bytes(const int offset, unsigned char* bytes, unsigned int nonce) {
    // nonce bytes
    unsigned char *nonce_ptr = (unsigned char *)&nonce;
    WRITE_NONCE_BYTE4;
}

#endif