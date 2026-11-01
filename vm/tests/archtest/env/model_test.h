#ifndef CALC_ARCHTEST_MODEL_TEST_H
#define CALC_ARCHTEST_MODEL_TEST_H

#define RVMODEL_BOOT

#define RVMODEL_HALT                                               \
    li a0, 0;                                                      \
    li a1, 0;                                                      \
    li a7, 0;                                                      \
    ecall;                                                         \
1:  j 1b;

#define RVMODEL_DATA_BEGIN                                         \
    .align 4; .globl begin_signature; begin_signature:

#define RVMODEL_DATA_END                                           \
    .align 4; .globl end_signature; end_signature:

#define RVMODEL_IO_INIT
#define RVMODEL_IO_WRITE_STR(_R, _STR)
#define RVMODEL_IO_CHECK()
#define RVMODEL_IO_ASSERT_GPR_EQ(_S, _R, _I)
#define RVMODEL_IO_ASSERT_SFPR_EQ(_F, _R, _I)
#define RVMODEL_IO_ASSERT_DFPR_EQ(_D, _R, _I)
#define RVMODEL_SET_MSW_INT
#define RVMODEL_CLEAR_MSW_INT
#define RVMODEL_CLEAR_MTIMER_INT
#define RVMODEL_CLEAR_MEXT_INT

#endif
