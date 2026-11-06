/* Linker script for riscv-tests binaries targeting the Hacash calc VM.
   CODE_BASE = 0x1000. PHDRS are explicit so lld emits exactly two PT_LOADs:
   an RX image (text+rodata) and a RW image (data+bss) above it. */
ENTRY(_start)
PHDRS
{
  text PT_LOAD FLAGS(5);  /* R+X */
  data PT_LOAD FLAGS(6);  /* R+W */
}
MEMORY
{
  TEXT (rx) : ORIGIN = 0x1000, LENGTH = 64K
  DATA (rw) : ORIGIN = 0x9000, LENGTH = 16K
}
SECTIONS
{
  .text   : { *(.text.init) *(.text .text.*) } > TEXT :text
  .rodata : { *(.rodata .rodata.* .srodata .srodata.*) } > TEXT :text
  .data   : { *(.data .data.* .sdata .sdata.*) } > DATA :data
  .bss    : { *(.sbss .sbss.* .bss .bss.* COMMON) } > DATA :data
  /DISCARD/ : { *(.eh_frame .eh_frame_hdr .eh_frame_entry .note* .riscv.attributes .comment .gnu*) }
}
