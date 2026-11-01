/* qemu-user oracle build: page-aligned segments (qemu maps PT_LOAD flags
   per page — .text must not share a page with read-only data). */
ENTRY(_start)
SECTIONS
{
  . = 0x10000;
  .text : { *(.oracle_start) *(.text.calc_entry) *(.text .text.*) }
  . = ALIGN(0x1000);
  .rodata : { *(.rodata .rodata.*) *(.srodata .srodata.*) *(.rodata.cst*) }
  . = ALIGN(0x1000);
  . = ALIGN(16);
  .data ALIGN(16) : { *(.data .data.*) *(.sdata .sdata.*) }
  .bss ALIGN(16) : { *(.sbss .sbss.*) *(.bss .bss.*) *(COMMON) }
}
