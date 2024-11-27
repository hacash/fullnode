#ifndef SHA3_256_CL
#define SHA3_256_CL

#define I64(x) x##UL
#define le2me_64(x) (x)
#define ROTL64(x, n) rotate(as_ulong(x), (n) & 0xFFFFFFFFFFFFFFFFUL)

/* constants */
#define NumberOfRounds 24
#define BLOCK_SIZE 136

// SHA3 (Keccak) constants for 24 rounds
__constant static const ulong keccak_round_constants[NumberOfRounds] = {
	I64(0x0000000000000001), I64(0x0000000000008082), I64(0x800000000000808A), I64(0x8000000080008000),
	I64(0x000000000000808B), I64(0x0000000080000001), I64(0x8000000080008081), I64(0x8000000000008009),
	I64(0x000000000000008A), I64(0x0000000000000088), I64(0x0000000080008009), I64(0x000000008000000A),
	I64(0x000000008000808B), I64(0x800000000000008B), I64(0x8000000000008089), I64(0x8000000000008003),
	I64(0x8000000000008002), I64(0x8000000000000080), I64(0x000000000000800A), I64(0x800000008000000A),
	I64(0x8000000080008081), I64(0x8000000000008080), I64(0x0000000080000001), I64(0x8000000080008008)
};

// Keccak pi() transformation
static inline void keccak_pi(__generic ulong A[25])
{
	ulong temp[25];

	#pragma unroll 25
    for (unsigned i = 0; i < 25; i++) {
        temp[i] = A[i];
    }

    A[ 0] = temp[ 0];
    A[ 1] = temp[ 6];
    A[ 2] = temp[12];
    A[ 3] = temp[18];
    A[ 4] = temp[24];
    A[ 5] = temp[ 3];
    A[ 6] = temp[ 9];
    A[ 7] = temp[10];
    A[ 8] = temp[16];
    A[ 9] = temp[22];
    A[10] = temp[ 1];
    A[11] = temp[ 7];
    A[12] = temp[13];
    A[13] = temp[19];
    A[14] = temp[20];
    A[15] = temp[ 4];
    A[16] = temp[ 5];
    A[17] = temp[11];
    A[18] = temp[17];
    A[19] = temp[23];
    A[20] = temp[ 2];
    A[21] = temp[ 8];
    A[22] = temp[14];
    A[23] = temp[15];
    A[24] = temp[21];
}

#define CHI_STEP(i) \
	A0 = A[0 + (i)]; \
	A1 = A[1 + (i)]; \
	A[0 + (i)] ^= ~A1 & A[2 + (i)]; \
	A[1 + (i)] ^= ~A[2 + (i)] & A[3 + (i)]; \
	A[2 + (i)] ^= ~A[3 + (i)] & A[4 + (i)]; \
	A[3 + (i)] ^= ~A[4 + (i)] & A0; \
	A[4 + (i)] ^= ~A0 & A1

// Keccak chi() transformation
static inline void keccak_chi(__generic ulong A[25])
{
	ulong A0, A1;
	CHI_STEP(0);
	CHI_STEP(5);
	CHI_STEP(10);
	CHI_STEP(15);
	CHI_STEP(20);
}

#define XORED_A(i) A[(i)] ^ A[(i) + 5] ^ A[(i) + 10] ^ A[(i) + 15] ^ A[(i) + 20]
#define THETA_STEP(i) \
	A[(i)]      ^= D[(i)]; \
	A[(i) + 5]  ^= D[(i)]; \
	A[(i) + 10] ^= D[(i)]; \
	A[(i) + 15] ^= D[(i)]; \
	A[(i) + 20] ^= D[(i)]

// Keccak theta() transformation
static inline void keccak_theta(__generic ulong A[25])
{
	ulong D[5];
	D[0] = ROTL64(XORED_A(1), 1) ^ XORED_A(4);
	D[1] = ROTL64(XORED_A(2), 1) ^ XORED_A(0);
	D[2] = ROTL64(XORED_A(3), 1) ^ XORED_A(1);
	D[3] = ROTL64(XORED_A(4), 1) ^ XORED_A(2);
	D[4] = ROTL64(XORED_A(0), 1) ^ XORED_A(3);
	THETA_STEP(0);
	THETA_STEP(1);
	THETA_STEP(2);
	THETA_STEP(3);
	THETA_STEP(4);
}

static inline void rhash_sha3_permutation(__generic ulong ALIGN state[25])
{
	ulong temp_state[25];
	for (unsigned round = 0; round < NumberOfRounds; round++)
	{
		keccak_theta(state);

		// apply Keccak rho() transformation
		temp_state[0] = state[0];
		temp_state[1] = ROTL64(state[1], 1);
		temp_state[2] = ROTL64(state[2], 62);
		temp_state[3] = ROTL64(state[3], 28);
		temp_state[4] = ROTL64(state[4], 27);
		temp_state[5] = ROTL64(state[5], 36);
		temp_state[6] = ROTL64(state[6], 44);
		temp_state[7] = ROTL64(state[7], 6);
		temp_state[8] = ROTL64(state[8], 55);
		temp_state[9] = ROTL64(state[9], 20);
		temp_state[10] = ROTL64(state[10], 3);
		temp_state[11] = ROTL64(state[11], 10);
		temp_state[12] = ROTL64(state[12], 43);
		temp_state[13] = ROTL64(state[13], 25);
		temp_state[14] = ROTL64(state[14], 39);
		temp_state[15] = ROTL64(state[15], 41);
		temp_state[16] = ROTL64(state[16], 45);
		temp_state[17] = ROTL64(state[17], 15);
		temp_state[18] = ROTL64(state[18], 21);
		temp_state[19] = ROTL64(state[19], 8);
		temp_state[20] = ROTL64(state[20], 18);
		temp_state[21] = ROTL64(state[21], 2);
		temp_state[22] = ROTL64(state[22], 61);
		temp_state[23] = ROTL64(state[23], 56);
		temp_state[24] = ROTL64(state[24], 14);

		#pragma unroll 25
		for (unsigned i = 0; i < 25; i++) {
            state[i] = temp_state[i];
        }

		keccak_pi(state);
		keccak_chi(state);

		// apply iota(state, round)
		*state ^= keccak_round_constants[round];
	}
}

inline void sha3_256_hash(const ulong *input, ulong *output)
{	
	ulong ALIGN hash[25] = {
		le2me_64(input[ 0]),
		le2me_64(input[ 1]),
		le2me_64(input[ 2]),
		le2me_64(input[ 3]),
		le2me_64(input[ 4]),
		le2me_64(input[ 5]),
		le2me_64(input[ 6]),
		le2me_64(input[ 7]),
		le2me_64(input[ 8]),
		le2me_64(input[ 9]),
		le2me_64(input[10]),
		le2me_64(0x0000000000000600),
		0,
		0,
		0,
		0,
		le2me_64(0x8000000000000000),
	};
	rhash_sha3_permutation(hash);
	output[0] = hash[0];
	output[1] = hash[1];
	output[2] = hash[2];
	output[3] = hash[3];
}

#endif  // SHA3_256_CL
