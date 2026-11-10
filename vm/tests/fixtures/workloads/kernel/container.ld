/* Link window only. Runtime mem_kib is chosen when the ELF is packed.
   0x80000 lets the secp256k1 image link; L1 admission is a separate check. */
ENTRY(calc_entry)
MEMORY
{
  IMAGE (rwx) : ORIGIN = 0x1000, LENGTH = 0x80000
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
}
