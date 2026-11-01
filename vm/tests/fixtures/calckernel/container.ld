/* Container layout: one contiguous exec+ro image (.text + .rodata), then
   initialized data. No page alignment — the calc container owns the map.
   Collect .sdata/.sbss/COMMON so small-data statics are not orphaned at 0.
   ALIGN(16) before `.data` so `.data`/`.bss` start at VM `data_base`
   (= align16(rodata_end)); the hole is empty and read-only. */
ENTRY(calc_entry)
SECTIONS
{
  . = 0x1000;
  .text : { *(.text.calc_entry) *(.text .text.*) }
  . = ALIGN(16);
  .rodata : {
    *(.rodata .rodata.*) *(.srodata .srodata.*) *(.rodata.cst*)
    /* rust-lld drops an empty .data and ignores ALIGN before it;
       keep the location 16-aligned inside .rodata so .bss follows
       at VM data_base. Trailing pad is read-only zeros. */
    . = ALIGN(16);
  }
  . = ALIGN(16);
  __calc_data_base = .;
  .data : ALIGN(16) { *(.data .data.*) *(.sdata .sdata.*) }
  .bss : ALIGN(16) { *(.sbss .sbss.*) *(.bss .bss.*) *(COMMON) }
}
