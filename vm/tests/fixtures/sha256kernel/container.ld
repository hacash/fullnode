/* PT_LOAD projection. IMAGE is the L1 guest window (64 KiB usable
   minus a 12 KiB host reserve for input+output+min stack).
   Link with -n so lld does not page-align PT_LOAD. */
ENTRY(calc_entry)
MEMORY
{
  IMAGE (rwx) : ORIGIN = 0x1000, LENGTH = 0xD000
}
PHDRS
{
  text PT_LOAD FLAGS(5);
  rodata PT_LOAD FLAGS(4);
  data PT_LOAD FLAGS(6);
}
SECTIONS
{
  . = ORIGIN(IMAGE);
  .text : { *(.text.calc_entry) *(.text .text.*) } :text
  .rodata : {
    *(.rodata .rodata.*) *(.srodata .srodata.*) *(.rodata.cst*)
  } :rodata
  .data : { *(.data .data.*) *(.sdata .sdata.*) } :data
  .bss : { *(.sbss .sbss.*) *(.bss .bss.*) *(COMMON) } :data
  /DISCARD/ : { *(.eh_frame) *(.eh_frame_hdr) }
  ASSERT(. <= ORIGIN(IMAGE) + LENGTH(IMAGE), "guest image overlaps L1 host window");
}
