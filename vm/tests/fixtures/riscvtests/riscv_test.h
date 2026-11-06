// riscv_test.h — calc VM edition (Hacash HVM calc coprocessor).
// Replaces the M-mode environment (no CSRs, no tohost): a test HALTs
// through the calc ecall ABI — a7=0, a0 = exit code (0 = pass), a1 = 0.
// The fail path reports the failing test number as the exit code, which
// the calc engine surfaces as GuestExit(exit_code).
#ifndef __RISCV_TEST_H
#define __RISCV_TEST_H

#define TESTNUM  x5

#define RVTEST_RV64U
#define RVTEST_RV32U

#define RVTEST_CODE_BEGIN                                               \
        .section .text.init,"ax",@progbits;                             \
        .align 2;                                                       \
        .globl _start;                                                  \
_start:

#define RVTEST_CODE_END                                                 \
        .align 2;

#define RVTEST_PASS                                                     \
        li a7, 0; li a0, 0; li a1, 0; ecall;

#define RVTEST_FAIL                                                     \
        li a7, 0; li a1, 0; mv a0, TESTNUM; ecall;                      \
        j .;

#define RVTEST_DATA_BEGIN                                               \
        .data; .align 3;

#define RVTEST_DATA_END

#endif
