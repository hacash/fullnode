

#define INPUT_BIG_LOCAL  do { \
		/*__local*/ sph_u32 *tp = &(T512_L[0]); \
		unsigned u, v; \
		m0 = 0; \
		m1 = 0; \
		m2 = 0; \
		m3 = 0; \
		m4 = 0; \
		m5 = 0; \
		m6 = 0; \
		m7 = 0; \
		m8 = 0; \
		m9 = 0; \
		mA = 0; \
		mB = 0; \
		mC = 0; \
		mD = 0; \
		mE = 0; \
		mF = 0; \
		for (u = 0; u < 8; u ++) { \
			unsigned db = buf(u); \
			for (v = 0; v < 8; v ++, db >>= 1) { \
				sph_u32 dm = SPH_T32(-(sph_u32)(db & 1)); \
				m0 ^= dm & *tp ++; \
				m1 ^= dm & *tp ++; \
				m2 ^= dm & *tp ++; \
				m3 ^= dm & *tp ++; \
				m4 ^= dm & *tp ++; \
				m5 ^= dm & *tp ++; \
				m6 ^= dm & *tp ++; \
				m7 ^= dm & *tp ++; \
				m8 ^= dm & *tp ++; \
				m9 ^= dm & *tp ++; \
				mA ^= dm & *tp ++; \
				mB ^= dm & *tp ++; \
				mC ^= dm & *tp ++; \
				mD ^= dm & *tp ++; \
				mE ^= dm & *tp ++; \
				mF ^= dm & *tp ++; \
			} \
		} \
	} while (0)



