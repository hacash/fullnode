/* qemu-user oracle build: page-aligned segments (qemu maps PT_LOAD flags
   per page — .text must not share a page with read-only data). */
ENTRY(_start)
SECTIONS
{
  . = 0x10000;
  .text : { *(.oracle_start) *(.text.calc_entry) *(.text .text.*) }
  . = ALIGN(0x1000);
  .rodata : { *(.rodata .rodata.*) }
  . = ALIGN(0x1000);
  .data : { *(.data .data.*) }
  .bss : { *(.bss .bss.*) }
}
