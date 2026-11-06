// rvc_ebreak.S — C.EBREAK trap probe for the calc C profile.
//
// C.EBREAK (bit12=1, rs2=0, rd=0 in quadrant 2) decodes to the same Trap
// instruction as 32-bit ebreak and ends execution by design, so it lives
// in its own image instead of the rvc_forms kernel. The runner asserts
// that the image aborts with CalcAbort::Trap (not GuestExit) — see
// vm/tests/calc_riscvtests.rs (`trap_expected`).

#include "riscv_test.h"

RVTEST_CODE_BEGIN
        c.ebreak
        j .
RVTEST_CODE_END
