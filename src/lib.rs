//! A procedural macro for generating PLL configuration parameters.
//!
//! The implementation is based on the Python script provided in the RP2040 SDK:
//! $PICO_SDK/src/rp2_common/hardware_clocks/scripts/vcocalc.py
//!
//! The macro takes a frequency (in kHz) as a literal and expands to an expression
//! of type `Option<PLLConfig>`.
//!
//! The algorithm searches over an expanded parameter space (REFDIV, FBDIV, PD1, and PD2)
//! using hard-coded defaults (e.g. a 12 MHz input, minimum reference frequency 5 MHz,
//! VCO limits between 750 and 1600 MHz) and selects the configuration with the smallest
//! error relative to the requested output frequency (converted from kHz to MHz).

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, LitInt};

// The defaultts are:
// * 12 MHz input,
// * 5 MHz minimum reference frequency,
// * VCO between 750 and 1600 MHz,
// * no locked REFDIV,
// * and default tie-break aka prefer the higher VCO.

const XOSC_MHZ: f64 = 12.0;
const REF_MIN: f64 = 5.0;
const VCO_MIN: f64 = 750.0;
const VCO_MAX: f64 = 1600.0;
const LOW_VCO: bool = false;
const LOCKED_REFDIV: Option<u8> = None;

mod pll {
    /// A simple newtype wrapper for a frequency in Hertz just to make it distinct from other values.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct HertzU32(pub u32);

    /// Extended PLL configuration parameters.
    #[derive(Debug, PartialEq)]
    pub struct PLLConfigExtended {
        /// Voltage Controlled Oscillator frequency (in Hz).
        pub vco_freq: HertzU32,
        /// Reference divider.
        pub refdiv: u8,
        /// Feedback divider.
        pub fbdiv: u16,
        /// Post Divider 1.
        pub post_div1: u8,
        /// Post Divider 2.
        pub post_div2: u8,
        /// Achieved system clock (output) frequency in MHz.
        pub sys_clk_mhz: f64,
    }

    /// Finds a PLL configuration by searching over an expanded parameter space.
    /// All frequencies here are in MHz except for the returned VCO frequency (in Hz).
    ///
    /// * `input_mhz`     - The input (reference) oscillator frequency (e.g. 12.0).
    /// * `requested_mhz` - The desired output frequency (e.g. 480.0).
    /// * `vco_min`       - Minimum allowed VCO frequency (e.g. 750.0).
    /// * `vco_max`       - Maximum allowed VCO frequency (e.g. 1600.0).
    /// * `ref_min`       - Minimum allowed reference frequency (e.g. 5.0).
    /// * `locked_refdiv` - If Some(n), restricts the search to REFDIV == n.
    /// * `low_vco`       - If true, among equally good solutions prefer the one with a lower VCO frequency;
    ///                     otherwise, prefer higher VCO.
    pub fn find_pll_config_extended(
        input_mhz: f64,
        requested_mhz: f64,
        vco_min: f64,
        vco_max: f64,
        ref_min: f64,
        locked_refdiv: Option<u8>,
        low_vco: bool,
    ) -> Option<PLLConfigExtended> {
        // Fixed ranges (as in the Python script)
        let fbdiv_range = 16..=320; // valid FBDIV values
        let postdiv_range = 1..=7; // valid post divider values

        // Allowed REFDIV values:
        // refdiv_min is fixed at 1; refdiv_max is defined here as 63.
        let refdiv_min: u8 = 1;
        let refdiv_max: u8 = 63;
        // Compute maximum allowed REFDIV based on the input frequency and minimum reference frequency.
        // (In the Python script: int(input / ref_min) is used.)
        let max_possible = ((input_mhz / ref_min).floor() as u8).min(refdiv_max);
        let max_refdiv = if max_possible < refdiv_min {
            refdiv_min
        } else {
            max_possible
        };

        // If locked, use just that value; otherwise, iterate from refdiv_min up through max_refdiv.
        let refdiv_iter: Box<dyn Iterator<Item = u8>> = if let Some(lock) = locked_refdiv {
            Box::new(std::iter::once(lock))
        } else {
            Box::new(refdiv_min..=max_refdiv)
        };

        // We'll track the best candidate as a tuple:
        // (achieved_out (MHz), fbdiv, pd1, pd2, refdiv, vco (MHz))
        let mut best: Option<(f64, u16, u8, u8, u8, f64)> = None;
        // Start with a relatively large error margin (here we use the requested frequency itself).
        let mut best_margin = requested_mhz;

        for refdiv in refdiv_iter {
            for fbdiv in fbdiv_range.clone() {
                // Compute VCO in MHz: vco = (input_mhz / refdiv) * fbdiv.
                let vco = (input_mhz / (refdiv as f64)) * (fbdiv as f64);
                if vco < vco_min || vco > vco_max {
                    continue;
                }
                // Loop over post divider combinations.
                for pd2 in postdiv_range.clone() {
                    for pd1 in postdiv_range.clone() {
                        let divider = (pd1 * pd2) as f64;
                        // Check that the VCO (scaled to kHz) divides exactly by the divider.
                        // (This ensures that the achieved output frequency is an integer value when computed in kHz.)
                        if (vco * 1000.0) % divider != 0.0 {
                            continue;
                        }
                        // Compute output frequency in MHz.
                        let out = vco / divider;
                        let margin = (out - requested_mhz).abs();

                        // Determine whether this candidate is “better.”
                        // In case of equal margin (within 1e-9) we compare the VCO frequency.
                        let update = if let Some((_, _, _, _, _, best_vco)) = best {
                            (margin < best_margin)
                                || ((margin - best_margin).abs() < 1e-9
                                    && (if low_vco {
                                        vco < best_vco
                                    } else {
                                        vco > best_vco
                                    }))
                        } else {
                            true
                        };

                        if update {
                            best_margin = margin;
                            best = Some((out, fbdiv, pd1, pd2, refdiv, vco));
                        }
                    }
                }
            }
        }

        best.map(|(out, fbdiv, pd1, pd2, refdiv, vco)| {
            // Compute VCO frequency in Hz.
            let vco_hz = (vco * 1_000_000.0).round() as u32;
            PLLConfigExtended {
                vco_freq: HertzU32(vco_hz),
                refdiv,
                fbdiv,
                post_div1: pd1,
                post_div2: pd2,
                sys_clk_mhz: out,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pll::find_pll_config_extended;

    struct TestCase {
        requested_mhz: f64,
        achieved_mhz: f64,
        expected_refdiv: u8,
        expected_fbdiv: u16,
        expected_pd1: u8,
        expected_pd2: u8,
        expected_vco: f64, // in MHz
    }

    #[test]
    fn test_pll_config_extended() {
        let test_cases = [
            // Requested: 480.0 MHz -> Expected: REFDIV=1, FBDIV=120, PD1=3, PD2=1, VCO=1440.0 MHz.
            TestCase {
                requested_mhz: 480.0,
                achieved_mhz: 480.0,
                expected_refdiv: 1,
                expected_fbdiv: 120,
                expected_pd1: 3,
                expected_pd2: 1,
                expected_vco: 1440.0,
            },
            // Requested: 250.0 MHz -> Expected: REFDIV=1, FBDIV=125, PD1=6, PD2=1, VCO=1500.0 MHz.
            TestCase {
                requested_mhz: 250.0,
                achieved_mhz: 250.0,
                expected_refdiv: 1,
                expected_fbdiv: 125,
                expected_pd1: 6,
                expected_pd2: 1,
                expected_vco: 1500.0,
            },
            // Requested: 176.0 MHz -> Expected: REFDIV=1, FBDIV=132, PD1=3, PD2=3, VCO=1584.0 MHz.
            TestCase {
                requested_mhz: 176.0,
                achieved_mhz: 176.0,
                expected_refdiv: 1,
                expected_fbdiv: 132,
                expected_pd1: 3,
                expected_pd2: 3,
                expected_vco: 1584.0,
            },
            // Requested: 130.0 MHz -> Expected: REFDIV=1, FBDIV=130, PD1=6, PD2=2, VCO=1560.0 MHz.
            TestCase {
                requested_mhz: 130.0,
                achieved_mhz: 130.0,
                expected_refdiv: 1,
                expected_fbdiv: 130,
                expected_pd1: 6,
                expected_pd2: 2,
                expected_vco: 1560.0,
            },
            // Requested: 32.0 MHz -> Expected: REFDIV=1, FBDIV=112, PD1=7, PD2=6, VCO=1344.0 MHz.
            TestCase {
                requested_mhz: 32.0,
                achieved_mhz: 32.0,
                expected_refdiv: 1,
                expected_fbdiv: 112,
                expected_pd1: 7,
                expected_pd2: 6,
                expected_vco: 1344.0,
            },
            // Requested: 20.0 MHz -> Expected: REFDIV=1, FBDIV=70, PD1=7, PD2=6, VCO=840.0 MHz.
            TestCase {
                requested_mhz: 20.0,
                achieved_mhz: 20.0,
                expected_refdiv: 1,
                expected_fbdiv: 70,
                expected_pd1: 7,
                expected_pd2: 6,
                expected_vco: 840.0,
            },
            // Requested: 125.0 MHz -> Expected: REFDIV=1, FBDIV=125, PD1=6, PD2=2, VCO=1500.0 MHz.
            TestCase {
                requested_mhz: 125.0,
                achieved_mhz: 125.0,
                expected_refdiv: 1,
                expected_fbdiv: 125,
                expected_pd1: 6,
                expected_pd2: 2,
                expected_vco: 1500.0,
            },
            // Requested: 48.0 MHz -> Expected: REFDIV=1, FBDIV=120, PD1=6, PD2=5, VCO=1440.0 MHz.
            TestCase {
                requested_mhz: 48.0,
                achieved_mhz: 48.0,
                expected_refdiv: 1,
                expected_fbdiv: 120,
                expected_pd1: 6,
                expected_pd2: 5,
                expected_vco: 1440.0,
            },
        ];

        for tc in &test_cases {
            let config = find_pll_config_extended(
                XOSC_MHZ,
                tc.requested_mhz,
                VCO_MIN,
                VCO_MAX,
                REF_MIN,
                LOCKED_REFDIV,
                LOW_VCO,
            )
            .unwrap_or_else(|| {
                panic!("No PLL config found for requested {} MHz", tc.requested_mhz)
            });

            // Recompute the achieved output frequency:
            // output = VCO / (pd1 * pd2)
            let achieved = (tc.expected_vco) / (config.post_div1 as f64 * config.post_div2 as f64);
            assert!(
                (achieved - tc.achieved_mhz).abs() < 1e-6,
                "Achieved frequency mismatch for {} MHz requested: got {} MHz, expected {} MHz",
                tc.requested_mhz,
                achieved,
                tc.achieved_mhz
            );
            // Check REFDIV, FBDIV, and post dividers.
            assert_eq!(
                config.refdiv, tc.expected_refdiv,
                "REFDIV mismatch for {} MHz requested",
                tc.requested_mhz
            );
            assert_eq!(
                config.fbdiv, tc.expected_fbdiv,
                "FBDIV mismatch for {} MHz requested",
                tc.requested_mhz
            );
            assert_eq!(
                config.post_div1, tc.expected_pd1,
                "PD1 mismatch for {} MHz requested",
                tc.requested_mhz
            );
            assert_eq!(
                config.post_div2, tc.expected_pd2,
                "PD2 mismatch for {} MHz requested",
                tc.requested_mhz
            );

            // Also check that the computed VCO equals the expected value.
            let computed_vco = XOSC_MHZ / (config.refdiv as f64) * (config.fbdiv as f64);
            assert!(
                (computed_vco - tc.expected_vco).abs() < 1e-6,
                "VCO mismatch for {} MHz requested: got {} MHz, expected {} MHz",
                tc.requested_mhz,
                computed_vco,
                tc.expected_vco
            );
        }
    }
}

/// The `pll_config` proc macro takes a frequency in kilohertz as a literal and
/// expands to an expression of type `Option<PLLConfig>`.
///
/// # Example
///
/// ```rust
/// use pico_pll_config::pll_config;
/// use rp2040_hal::pll::PLLConfig;
///
/// // 480000 represents 480 MHz (i.e. 480000 kHz)
/// let config = pll_config!(480000);
/// const CONFIG: PLLConfig = pll_config!(480000).unwrap();
/// ```
#[proc_macro]
pub fn pll_config(input: TokenStream) -> TokenStream {
    // Parse the input as an integer literal.
    let input_lit = parse_macro_input!(input as LitInt);
    let freq_khz: u64 = input_lit.base10_parse().expect("Invalid integer literal");

    let requested_mhz = freq_khz as f64 / 1000.0;
    let result = pll::find_pll_config_extended(
        XOSC_MHZ,
        requested_mhz,
        VCO_MIN,
        VCO_MAX,
        REF_MIN,
        LOCKED_REFDIV,
        LOW_VCO,
    );

    let expanded = if let Some(ref config) = result {
        let vco_mhz = config.vco_freq.0 / 1_000_000;
        let refdiv = config.refdiv;
        let post_div1 = config.post_div1;
        let post_div2 = config.post_div2;
        quote! {
            Some(rp2040_hal::pll::PLLConfig {
                vco_freq: fugit::HertzU32::MHz(#vco_mhz),
                refdiv: #refdiv,
                post_div1: #post_div1,
                post_div2: #post_div2,
            })
        }
    } else {
        quote! { None }
    };

    TokenStream::from(expanded)
}
