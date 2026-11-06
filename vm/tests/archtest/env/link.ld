OUTPUT_ARCH(riscv)
ENTRY(rvtest_entry_point)
MEMORY
{
  IMAGE (rwx) : ORIGIN = 0x1000, LENGTH = 0xD000
}
PHDRS
{
  text PT_LOAD FLAGS(5);
  data PT_LOAD FLAGS(6);
}
SECTIONS
{
  . = ORIGIN(IMAGE);
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
  ASSERT(. <= ORIGIN(IMAGE) + LENGTH(IMAGE), "guest image overlaps L1 host window");
}
