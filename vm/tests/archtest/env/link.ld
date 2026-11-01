OUTPUT_ARCH(riscv)
ENTRY(rvtest_entry_point)
PHDRS
{
  text PT_LOAD FLAGS(5);
  data PT_LOAD FLAGS(6);
}
SECTIONS
{
  . = 0x1000;
  .text : {
    *(.text.init)
    *(.text .text.*)
  } :text
  . = ALIGN(16);
  .data : {
    *(.rodata .rodata.*)
    *(.srodata .srodata.*)
    *(.data .data.*)
    *(.sdata .sdata.*)
    *(.bss .sbss .sbss.*)
    *(COMMON)
  } :data
  PROVIDE(__global_pointer$ = . + 0x800);
}
