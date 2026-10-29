/* Container layout: one contiguous exec+ro image (.text + .rodata), then
   initialized data. No page alignment — the calc container owns the map. */
ENTRY(calc_entry)
SECTIONS
{
  . = 0x0;
  .text : { *(.text.calc_entry) *(.text .text.*) }
  . = ALIGN(16);
  .rodata : { *(.rodata .rodata.*) }
  .data : { *(.data .data.*) }
  .bss : { *(.bss .bss.*) }
}
