// rvc_unimp.S — all-zero c.unimp halfword trap probe for the calc C profile.
//
// The all-zero halfword is the canonical C.UNIMP encoding (LLVM emits it on
// dead paths); the profile-C decoder admits it as a Trap instruction so an
// RVC image still verifies, and it ends execution like ebreak. It lives in
// its own image for the same reason as rvc_ebreak.S — see
// vm/tests/calc_riscvtests.rs (`trap_expected`).

#include "riscv_test.h"

RVTEST_CODE_BEGIN
        .half 0x0000
        j .
RVTEST_CODE_END
