# A procedural macro for generating PLL configuration parameters.

The implementation is based on the Python script provided in the RP SDK:
`$PICO_SDK/src/rp2_common/hardware_clocks/scripts/vcocalc.py`

The algorithm searches over an expanded parameter space (REFDIV, FBDIV, PD1, and PD2)
using hard-coded defaults (e.g. a 12 MHz input, minimum reference frequency 5 MHz,
VCO limits between 750 and 1600 MHz) and selects the configuration with the smallest
error relative to the requested output frequency (converted from kHz to MHz).

The `pll_config` proc macro takes a frequency in kilohertz as a literal and
expands to an expression of type `Option<PLLConfig>`:

```rust
use pico_pll_config::pll_config;
use rp2040_hal::pll::PLLConfig;

// 480000 represents 480 MHz (i.e. 480000 kHz)
let config = pll_config!(480000);
const CONFIG: PLLConfig = pll_config!(480000).unwrap();
```
