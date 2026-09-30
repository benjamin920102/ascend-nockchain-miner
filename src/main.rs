use std::{
    ffi::{c_char, c_int, c_void, CStr},
    process::ExitCode,
    ptr::NonNull,
    sync::{Arc, Mutex},
};

use ai_pow::{
    matmul::TileState,
    pearl_compat::pearl_jackpot_hash,
    tile_hash::hash_le_target,
};
use ai_pow_miner::{
    cli::{init_tracing, CommonArgs},
    run::{run_reference_with_backend, run_with_backend, MinerError},
    search::{MeteredSearchBackend, SearchBackend, SearchBackendError, SearchBatch, SearchWinner},
};
use anyhow::{bail, Result};
use clap::Parser;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

unsafe extern "C" {
    fn ascend_miner_is_hardware() -> c_int;
    fn ascend_miner_create(device: c_int, out: *mut *mut c_void) -> c_int;
    fn ascend_miner_destroy(ctx: *mut c_void);
    fn ascend_miner_tile_state(
        ctx: *mut c_void,
        a: *const i8,
        b: *const i8,
        h: u32,
        w: u32,
        k: u32,
        rank: u32,
        dot_len: u32,
        out: *mut i32,
    ) -> c_int;
    fn ascend_miner_error_string(code: c_int) -> *const c_char;
}

fn ffi_error(code: c_int) -> String {
    let p = unsafe { ascend_miner_error_string(code) };
    if p.is_null() { format!("Ascend error {code}") }
    else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

#[derive(Debug)]
struct Handle(NonNull<c_void>);
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) { unsafe { ascend_miner_destroy(self.0.as_ptr()) } }
}

#[derive(Debug)]
struct AscendBackend {
    device: usize,
    batch_attempts: u64,
    hardware: bool,
    handle: Handle,
    dispatch: Mutex<()>,
}

impl AscendBackend {
    fn new(device: usize, batch_attempts: u64, require_hardware: bool) -> Result<Self> {
        if batch_attempts == 0 { bail!("--batch-attempts must be nonzero"); }
        let device_id = c_int::try_from(device)?;
        let hardware = unsafe { ascend_miner_is_hardware() != 0 };
        if require_hardware && !hardware {
            bail!("CPU stub build; rebuild with `make build`");
        }
        let mut raw = std::ptr::null_mut();
        let status = unsafe { ascend_miner_create(device_id, &mut raw) };
        if status != 0 { bail!("Ascend init failed: {}", ffi_error(status)); }
        Ok(Self {
            device,
            batch_attempts,
            hardware,
            handle: Handle(NonNull::new(raw).ok_or_else(|| anyhow::anyhow!("null Ascend context"))?),
            dispatch: Mutex::new(()),
        })
    }

    fn tile(
        &self,
        a: &[i8], b: &[i8], h: usize, w: usize, k: usize, rank: usize, dot_len: usize,
    ) -> Result<TileState, SearchBackendError> {
        if a.len() != h.saturating_mul(k) || b.len() != w.saturating_mul(k) {
            return Err(SearchBackendError::BackendUnavailable("invalid prepared matrix shape".into()));
        }
        let mut state = [0i32; 16];
        let status = unsafe {
            ascend_miner_tile_state(
                self.handle.0.as_ptr(), a.as_ptr(), b.as_ptr(),
                u32::try_from(h).map_err(unavailable)?,
                u32::try_from(w).map_err(unavailable)?,
                u32::try_from(k).map_err(unavailable)?,
                u32::try_from(rank).map_err(unavailable)?,
                u32::try_from(dot_len).map_err(unavailable)?,
                state.as_mut_ptr(),
            )
        };
        if status != 0 {
            return Err(SearchBackendError::BackendUnavailable(format!("Ascend kernel: {}", ffi_error(status))));
        }
        Ok(TileState(state))
    }
}

fn unavailable(e: impl std::fmt::Display) -> SearchBackendError {
    SearchBackendError::BackendUnavailable(e.to_string())
}

impl SearchBackend for AscendBackend {
    fn search_dense(
        &self,
        template: Arc<ai_pow::pearl_compat::PreparedPearlPatternJob>,
        batch: SearchBatch,
    ) -> Result<Option<SearchWinner>, SearchBackendError> {
        let _lock = self.dispatch.lock().map_err(|_| unavailable("dispatch lock poisoned"))?;
        let params = template.params();
        let config = template.config();
        let h = config.rows_pattern.size().map_err(SearchBackendError::DenseEvaluation)? as usize;
        let w = config.cols_pattern.size().map_err(SearchBackendError::DenseEvaluation)? as usize;
        let dot_len = config.dot_product_length().map_err(SearchBackendError::DenseEvaluation)? as usize;
        let mut scratch = template.scratch();

        for ordinal in batch.start..batch.end_exclusive() {
            let (row, col) = template.offsets_at_ordinal(ordinal)
                .ok_or(SearchBackendError::DenseOrdinalOutOfRange(ordinal))?;
            let (a, b) = template.prepare_offset(row, col, &mut scratch)?;
            let state = self.tile(a, b, h, w, params.k as usize, params.noise_rank as usize, dot_len)?;
            let jackpot = pearl_jackpot_hash(&state, &template.commitments().s_a);
            if hash_le_target(&jackpot, &batch.threshold) {
                return Ok(Some(SearchWinner { ordinal, jackpot_hash: jackpot }));
            }
        }
        Ok(None)
    }

    fn search_reference(
        &self,
        template: Arc<ai_pow_miner::reference::PreparedReferenceMoeTemplate>,
        batch: SearchBatch,
    ) -> Result<Option<SearchWinner>, SearchBackendError> {
        let _lock = self.dispatch.lock().map_err(|_| unavailable("dispatch lock poisoned"))?;
        let config = template.config();
        let (routing, inner, local_b, _, _) = template.schedule();
        let h = routing.outer_indices(0, inner).map_err(unavailable)?.len();
        let w = local_b.len();
        let mut scratch = template.scratch();

        for ordinal in batch.start..batch.end_exclusive() {
            let extranonce = u32::try_from(ordinal)
                .map_err(|_| SearchBackendError::CanonicalOrdinalOutOfRange(ordinal))?;
            let commitments = template.prepare_attempt(extranonce, &mut scratch);
            let (a, b) = template.prepared_strips(&scratch);
            let state = self.tile(a, b, h, w, config.common_dim as usize, config.rank as usize, config.common_dim as usize)?;
            let jackpot = pearl_jackpot_hash(&state, &commitments.s_a);
            if hash_le_target(&jackpot, &batch.threshold) {
                return Ok(Some(SearchWinner { ordinal, jackpot_hash: jackpot }));
            }
        }
        Ok(None)
    }

    fn batch_attempts(&self) -> u64 { self.batch_attempts }
}

#[derive(Parser, Debug)]
#[command(name = "ascend-nockchain-miner", about = "Nockchain AI-PoW miner for Ascend 910B/CANN", version)]
struct Args {
    #[command(flatten)]
    common: CommonArgs,
    #[arg(long, default_value_t = 0)]
    device: usize,
    #[arg(long, default_value_t = 256)]
    batch_attempts: u64,
}

fn run(args: Args) -> Result<()> {
    if !args.common.reference && !args.common.dense_production {
        bail!("Ascend v0 supports --reference or --dense-production");
    }
    let backend = AscendBackend::new(args.device, args.batch_attempts, true)?;
    info!(device = backend.device, batch_attempts = backend.batch_attempts, hardware = backend.hardware, "Ascend backend ready");
    let backend: Arc<dyn SearchBackend> = MeteredSearchBackend::new("ascend910b", Arc::new(backend));
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let shutdown = CancellationToken::new();
    let signal = shutdown.clone();
    runtime.spawn(async move { if tokio::signal::ctrl_c().await.is_ok() { signal.cancel(); } });

    let result = if args.common.reference {
        runtime.block_on(run_reference_with_backend(
            args.common.node_addr.clone(), args.common.mining_pkh_configs()?, shutdown, backend,
        ))
    } else {
        runtime.block_on(run_with_backend(args.common.build_miner_config()?, shutdown, backend))
    };
    match result {
        Ok(()) => Ok(()),
        Err(MinerError::TooManyReconnects { count }) => bail!("gave up after {count} reconnect failures"),
        Err(e) => Err(e.into()),
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    init_tracing(&args.common.log);
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => { error!(error = %e, "miner stopped"); ExitCode::from(1) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_pow::matmul::{compute_pattern_tile_state_from_slices, PatternTileScratch};

    #[test]
    fn stub_matches_rust_oracle() {
        let backend = AscendBackend::new(0, 8, false).unwrap();
        let (h, w, k, rank) = (3usize, 5usize, 64usize, 8usize);
        let a: Vec<i8> = (0..h*k).map(|i| ((i*17%101) as i16 - 50) as i8).collect();
        let b: Vec<i8> = (0..w*k).map(|i| ((i*29%97) as i16 - 48) as i8).collect();
        let got = backend.tile(&a, &b, h, w, k, rank, k).unwrap();
        let mut scratch = PatternTileScratch::new(h, w);
        let want = compute_pattern_tile_state_from_slices(&a, &b, h, w, k, rank, k, &mut scratch);
        assert_eq!(got, want);
    }
}
