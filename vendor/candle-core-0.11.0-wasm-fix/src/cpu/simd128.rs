use super::{Cpu, CpuBF16, CpuF16};
use core::arch::wasm32::*;
use half::{bf16, f16};

pub struct CurrentCpu {}

const STEP: usize = 16;
const EPR: usize = 4;
const ARR: usize = STEP / EPR;

impl Cpu for CurrentCpu {
    type Unit = v128;
    type Array = [v128; ARR];

    const STEP: usize = STEP;
    const EPR: usize = EPR;
    const ARR: usize = ARR;

    unsafe fn zero() -> Self::Unit {
        f32x4_splat(0.0)
    }

    unsafe fn zero_array() -> Self::Array {
        [Self::zero(); ARR]
    }

    unsafe fn from_f32(v: f32) -> Self::Unit {
        f32x4_splat(v)
    }

    unsafe fn load(mem_addr: *const f32) -> Self::Unit {
        v128_load(mem_addr as *mut v128)
    }

    unsafe fn vec_add(a: Self::Unit, b: Self::Unit) -> Self::Unit {
        f32x4_add(a, b)
    }

    unsafe fn vec_fma(a: Self::Unit, b: Self::Unit, c: Self::Unit) -> Self::Unit {
        f32x4_add(f32x4_mul(b, c), a)
    }

    unsafe fn vec_store(mem_addr: *mut f32, a: Self::Unit) {
        v128_store(mem_addr as *mut v128, a);
    }

    unsafe fn vec_reduce(mut x: Self::Array, y: *mut f32) {
        for i in 0..ARR / 2 {
            x[2 * i] = f32x4_add(x[2 * i], x[2 * i + 1]);
        }
        for i in 0..ARR / 4 {
            x[4 * i] = f32x4_add(x[4 * i], x[4 * i + 2]);
        }
        for i in 0..ARR / 8 {
            x[8 * i] = f32x4_add(x[8 * i], x[8 * i + 4]);
        }
        *y = f32x4_extract_lane::<0>(x[0])
            + f32x4_extract_lane::<1>(x[0])
            + f32x4_extract_lane::<2>(x[0])
            + f32x4_extract_lane::<3>(x[0]);
    }
}

// Upstream gap (candle-core 0.11.0): unlike the AVX2 and NEON backends, this module never
// defined `CurrentCpuBF16`/`CurrentCpuF16` — but `super::mod.rs`'s bf16/f16 matmul kernels
// reference them unconditionally whenever `simd128` is active, so the crate simply fails to
// compile with `target-feature=+simd128` on wasm32 (`error[E0433]: cannot find type
// CurrentCpuBF16`). These are plain scalar (`Unit = f32`, one element at a time) — not real
// v128 SIMD — restoring correctness without hand-writing new bf16/f16 wasm SIMD intrinsics,
// which is not something to improvise for a PII-detection inference path. `CurrentCpu` above
// (the f32 matmul path, which is what "2-4x speedup" in this repo's `.cargo/config.toml`
// actually refers to) is unaffected and keeps its real f32x4 vectorization; only the
// lower-precision bf16/f16 kernels lose SIMD acceleration relative to AVX2/NEON.

pub struct CurrentCpuBF16 {}

impl CpuBF16 for CurrentCpuBF16 {
    type Unit = f32;
    type Array = [f32; 1];

    const STEP: usize = 1;
    const EPR: usize = 1;

    unsafe fn zero() -> Self::Unit {
        0f32
    }

    unsafe fn zero_array() -> Self::Array {
        [0f32; 1]
    }

    unsafe fn from_f32(v: f32) -> Self::Unit {
        v
    }

    unsafe fn load(mem_addr: *const bf16) -> Self::Unit {
        (*mem_addr).to_f32()
    }

    unsafe fn vec_add(a: Self::Unit, b: Self::Unit) -> Self::Unit {
        a + b
    }

    unsafe fn vec_fma(a: Self::Unit, b: Self::Unit, c: Self::Unit) -> Self::Unit {
        b * c + a
    }

    unsafe fn vec_store(mem_addr: *mut bf16, a: Self::Unit) {
        *mem_addr = bf16::from_f32(a);
    }

    unsafe fn vec_reduce(x: Self::Array, y: *mut f32) {
        *y = x[0];
    }
}

pub struct CurrentCpuF16 {}

impl CpuF16 for CurrentCpuF16 {
    type Unit = f32;
    type Array = [f32; 1];

    const STEP: usize = 1;
    const EPR: usize = 1;

    unsafe fn zero() -> Self::Unit {
        0f32
    }

    unsafe fn zero_array() -> Self::Array {
        [0f32; 1]
    }

    unsafe fn from_f32(v: f32) -> Self::Unit {
        v
    }

    unsafe fn load(mem_addr: *const f16) -> Self::Unit {
        (*mem_addr).to_f32()
    }

    unsafe fn vec_add(a: Self::Unit, b: Self::Unit) -> Self::Unit {
        a + b
    }

    unsafe fn vec_fma(a: Self::Unit, b: Self::Unit, c: Self::Unit) -> Self::Unit {
        b * c + a
    }

    unsafe fn vec_store(mem_addr: *mut f16, a: Self::Unit) {
        *mem_addr = f16::from_f32(a);
    }

    unsafe fn vec_reduce(x: Self::Array, y: *mut f32) {
        *y = x[0];
    }
}
