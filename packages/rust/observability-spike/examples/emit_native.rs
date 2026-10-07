//! Point 6 baseline: the same `spike_emit` the .NET harness calls, without .NET in the process.

use observability_spike::ffi::{
    spike_emit, spike_runtime_free, spike_runtime_new, spike_set_native_count,
};

fn main() {
    unsafe {
        let runtime = spike_runtime_new();
        assert_eq!(spike_set_native_count(runtime), 0);
        spike_emit(runtime, 1, 20_000);
        for threads in [1u32, 4] {
            let mut samples: Vec<f64> = (0..5)
                .map(|_| spike_emit(runtime, threads, 500_000) as f64 / (500_000.0 * threads as f64))
                .collect();
            samples.sort_by(f64::total_cmp);
            println!(
                "emit_native threads={threads}: wall ns per event min {:.0} median {:.0} max {:.0}",
                samples[0], samples[2], samples[4]
            );
        }
        spike_runtime_free(runtime);
    }
}
