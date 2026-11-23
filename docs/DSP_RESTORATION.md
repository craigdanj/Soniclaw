# Audio restoration primitives

Prepare Denoiser with a power-of-two FFT size (256–4096), a subtraction strength (0–4), and a minimum amplitude gain (0.01–1). Feed a representative noise-only lead-in in Learn mode. Set Frozen to apply it to program material. Adaptive also tracks lower-energy changes slowly. Use multi-second noise-only rehearsal material and compare with the original. Strong attenuation can remove quiet wanted content or leave musical noise. Reset clears overlap history while retaining the learned profile. Switching back to Learn clears the profile. Algorithmic delay is one FFT window.

Dehum prepares a 40–70 Hz harmonic notch cascade with 1–24 harmonics and Q 5–100. Use 50 or 60 Hz for stable mains hum. Wider filters cover drift at the cost of removing more useful frequency content. declick is a one-pass offline repair for isolated outliers with quiet neighboring differences. Adjust the threshold with a short audition. Intentional attacks and extended crackle may need manual correction.

Prepare or design on the control thread. Use a single owner for runtime state. Denoising covers up to eight channels and hum or click processing covers up to 64. Callback denoising uses preplanned FFTs and no allocations. Reset for independent takes and persist user settings in application presets. Relearn profiles after rate/window changes. Earlier presets are unaffected. Invalid denoiser callback shapes clear output, and invalid hum/click buffers reject before processing. The fixture measures noise-only attenuation on a synthetic source. Tests also measure steady mains rejection and channel-specific click repair.

Run `cargo run -p soniclaw-dsp-suite --locked --offline -- restoration NEW_DIRECTORY`. [Executed checks](validation/v0_108_0/REPORT.md).
