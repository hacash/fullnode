// rvc_forms.S — calc-native RV64C conformance kernel.
//
// Upstream rvc.S is excluded from this corpus (see build.sh): its first
// case embeds .dword data inside the executable segment, which the calc
// verifier deliberately rejects (every X-segment byte must decode to a
// whitelisted instruction). This kernel instead exercises EVERY whitelisted
// compressed form of the C profile with one checked case apiece — each
// result is compared against its expected value, and a mismatch exits
// through the fail path with the case number as the calc exit code:
//
//   101 c.addi4spn   111 c.lui        121 c.j          125 c.swsp+c.lwsp
//   102 c.ld         112 c.srli       122 c.beqz       126 c.sdsp+c.ldsp
//   103 c.lw         113 c.srai       123 c.bnez       129 c.jr
//   104 c.sd         114 c.andi       124 c.slli       130 c.mv
//   105 c.sw         115 c.sub        131 c.jalr
//   106 c.nop        116 c.xor        132 c.add
//   107 c.addi       117 c.or
//   108 c.addiw      118 c.and
//   109 c.li         119 c.subw
//   110 c.addi16sp   120 c.addw
//
// The trap encodings (c.ebreak, the all-zero c.unimp halfword) live in the
// separate rvc_ebreak / rvc_unimp images: they end execution by design.
//
// The compressed-frame cases lower sp by 64 bytes; the engine guarantees
// CALC_STACK_SIZE (2 KiB) of free RW below the guest image, so the frame
// is always backed by real memory.

#include "riscv_test.h"
#include "test_macros.h"

RVTEST_CODE_BEGIN
        // ---- 101: C.ADDI4SPN (address arithmetic into rd')
        li      TESTNUM, 101
        c.addi4spn x12, sp, 24
        addi    t2, sp, 24
        bne     x12, t2, fail

        // ---- 102: C.LD from the data segment
        li      TESTNUM, 102
        la      x14, scratch_d
        c.ld    x12, 0(x14)
        li      t2, 0x1122334455667788
        bne     x12, t2, fail

        // ---- 103: C.LW (both offsets; second word sign-extends)
        li      TESTNUM, 103
        la      x14, scratch_w
        c.lw    x12, 0(x14)
        li      t2, 0x12345678
        bne     x12, t2, fail
        c.lw    x13, 4(x14)
        li      t2, 0xffffffff9abcdef0
        bne     x13, t2, fail

        // ---- 104: C.SD into the data segment, read back wide
        li      TESTNUM, 104
        la      x15, out_d
        li      x12, 0x0fedcba987654321
        c.sd    x12, 0(x15)
        ld      t2, 0(x15)
        bne     x12, t2, fail

        // ---- 105: C.SW
        li      TESTNUM, 105
        la      x14, out_w
        li      x12, 0x13579bdf
        c.sw    x12, 0(x14)
        lw      t2, 0(x14)
        bne     x12, t2, fail

        // ---- 106: C.NOP (executes as addi x0, x0, 0)
        li      TESTNUM, 106
        c.nop

        // ---- 107: C.ADDI (negative immediate)
        li      TESTNUM, 107
        li      x10, 5
        c.addi  x10, -12
        li      t2, -7
        bne     x10, t2, fail

        // ---- 108: C.ADDIW (32-bit wrap sign-extends to 64)
        li      TESTNUM, 108
        li      x10, 0x7fffffff
        c.addiw x10, 1
        li      t2, 0xffffffff80000000
        bne     x10, t2, fail

        // ---- 109: C.LI both immediate signs
        li      TESTNUM, 109
        c.li    x10, -31
        li      t2, -31
        bne     x10, t2, fail
        c.li    x13, 31
        li      t2, 31
        bne     x13, t2, fail

        // ---- 110: C.ADDI16SP (min, near-max and unit immediates; sp restored)
        li      TESTNUM, 110
        mv      t3, sp
        c.addi16sp sp, -512
        addi    t2, t3, -512
        bne     sp, t2, fail
        c.addi16sp sp, 496
        addi    t2, t3, -16
        bne     sp, t2, fail
        c.addi16sp sp, 16
        bne     sp, t3, fail

        // ---- 111: C.LUI (nonzero rd; the assembler takes a negative imm as
        // its 20-bit two-s-complement encoding)
        li      TESTNUM, 111
        c.lui   x10, 31
        li      t2, 0x1f000
        bne     x10, t2, fail
        c.lui   x11, 0xfffff
        li      t2, 0xfffffffffffff000
        bne     x11, t2, fail

        // ---- 112: C.SRLI (logical; incl shamt >= 32)
        li      TESTNUM, 112
        li      x10, 0xf0000000f
        c.srli  x10, 4
        li      t2, 0xf0000000
        bne     x10, t2, fail
        li      x10, 0x3000000001
        c.srli  x10, 36
        li      t2, 3
        bne     x10, t2, fail

        // ---- 113: C.SRAI (arithmetic)
        li      TESTNUM, 113
        li      x10, -16
        c.srai  x10, 2
        li      t2, -4
        bne     x10, t2, fail

        // ---- 114: C.ANDI
        li      TESTNUM, 114
        li      x10, 0xff
        c.andi  x10, 0x0f
        li      t2, 0x0f
        bne     x10, t2, fail

        // ---- 115..118: C.SUB / C.XOR / C.OR / C.AND
        li      TESTNUM, 115
        li      x10, 10
        li      x11, 4
        c.sub   x10, x11
        li      t2, 6
        bne     x10, t2, fail
        li      x10, 10
        c.xor   x10, x11
        li      t2, 14
        bne     x10, t2, fail
        li      x10, 10
        c.or    x10, x11
        li      t2, 14
        bne     x10, t2, fail
        li      x10, 10
        c.and   x10, x11
        li      t2, 0
        bne     x10, t2, fail

        // ---- 119: C.SUBW (32-bit wrap sign-extends)
        li      TESTNUM, 119
        li      x10, 5
        li      x11, 9
        c.subw  x10, x11
        li      t2, -4
        bne     x10, t2, fail

        // ---- 120: C.ADDW
        li      TESTNUM, 120
        li      x10, 0x7fffffff
        li      x11, 1
        c.addw  x10, x11
        li      t2, 0xffffffff80000000
        bne     x10, t2, fail

        // ---- 121: C.J (unconditional, skips the poison store)
        li      TESTNUM, 121
        li      x10, 0
        c.j     1f
        li      x10, 0xbad
1:
        li      t2, 0
        bne     x10, t2, fail

        // ---- 122: C.BEQZ not-taken and taken paths
        li      TESTNUM, 122
        li      x10, 0
        li      x11, 1
        c.beqz  x11, 2f          // nonzero -> falls into the poison
        li      x10, 0xbad
2:
        li      t2, 0xbad
        bne     x10, t2, fail
        li      x10, 0
        c.beqz  x10, 3f          // zero -> skips the poison
        li      x10, 0xbad
3:
        li      t2, 0
        bne     x10, t2, fail

        // ---- 123: C.BNEZ taken and not-taken paths
        li      TESTNUM, 123
        li      x10, 5
        c.bnez  x10, 4f          // nonzero -> skips the poison
        li      x10, 0xbad
4:
        li      t2, 5
        bne     x10, t2, fail
        li      x10, 0
        c.bnez  x10, 5f          // zero -> must NOT branch
        j       6f
5:      li      x10, 0xbad
6:
        li      t2, 0
        bne     x10, t2, fail

        // ---- 124: C.SLLI (incl shamt >= 32)
        li      TESTNUM, 124
        li      x10, 3
        c.slli  x10, 5
        li      t2, 96
        bne     x10, t2, fail
        li      x10, 1
        c.slli  x10, 32
        li      t2, 0x100000000
        bne     x10, t2, fail

        // ---- 125: C.SWSP + C.LWSP inside a lowered sp frame
        li      TESTNUM, 125
        mv      t3, sp
        c.addi16sp sp, -64
        li      x10, 0x2468ace0
        c.swsp  x10, 8(sp)
        c.lwsp  x12, 8(sp)
        bne     x12, x10, fail

        // ---- 126: C.SDSP + C.LDSP in the same frame
        li      TESTNUM, 126
        li      x11, 0x135724689abc
        c.sdsp  x11, 16(sp)
        c.ldsp  x12, 16(sp)
        bne     x12, x11, fail
        c.addi16sp sp, 64
        bne     sp, t3, fail

        // ---- 129: C.JR (indirect, skips the poison store)
        li      TESTNUM, 129
        li      x10, 0
        la      x14, Ljr
        c.jr    x14
        li      x10, 0xbad
Ljr:
        li      t2, 0
        bne     x10, t2, fail

        // ---- 130: C.MV
        li      TESTNUM, 130
        li      x11, 1234
        c.mv    x10, x11
        li      t2, 1234
        bne     x10, t2, fail

        // ---- 131: C.JALR (ra = pc + 2, not pc + 4)
        li      TESTNUM, 131
        li      x10, 0
        la      x14, Ljalr
        c.jalr  x14
Lpoison:
        li      x10, 0xbad
Ljalr:
        li      x11, 77
        la      t2, Lpoison
        bne     x1, t2, fail     // ra must point just past the 2-byte jalr
        li      t2, 0
        bne     x10, t2, fail    // poison skipped
        li      t2, 77
        bne     x11, t2, fail

        // ---- 132: C.ADD
        li      TESTNUM, 132
        li      x10, 40
        li      x11, 2
        c.add   x10, x11
        li      t2, 42
        bne     x10, t2, fail

        TEST_PASSFAIL

RVTEST_CODE_END

RVTEST_DATA_BEGIN
scratch_w:      .word   0x12345678
                .word   0x9abcdef0
scratch_d:      .dword  0x1122334455667788
out_w:          .word   0
out_d:          .dword  0
RVTEST_DATA_END
