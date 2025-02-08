# A procedural macro for generating PLL configuration parameters.

The implementation is based on the Python script provided in the RP SDK:
`$PICO_SDK/src/rp2_common/hardware_clocks/scripts/vcocalc.py`

The macro takes a frequency (in kHz) as a literal and expands to an expression
of type `Option<PLLConfig>`.

The algorithm searches over an expanded parameter space (REFDIV, FBDIV, PD1, and PD2)
using hard-coded defaults (e.g. a 12 MHz input, minimum reference frequency 5 MHz,
VCO limits between 750 and 1600 MHz) and selects the configuration with the smallest
error relative to the requested output frequency (converted from kHz to MHz).
