use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;

pub fn create_session_from_bytes(
    model_bytes: &[u8],
    model_name: &str,
    optimization_level: GraphOptimizationLevel,
    intra_threads: Option<usize>,
) -> anyhow::Result<Session> {
    #[cfg(target_os = "windows")]
    {
        let gpu_result = (|| -> ort::Result<Session> {
            let mut builder = Session::builder()?;
            use ort::ep::DirectML;
            builder = builder.with_execution_providers([DirectML::default().build()])?;
            builder = builder.with_optimization_level(optimization_level)?;
            if let Some(threads) = intra_threads {
                builder = builder.with_intra_threads(threads)?;
            }
            builder.commit_from_memory(model_bytes)
        })();

        if let Ok(session) = gpu_result {
            println!("[OK] Модель '{model_name}' запущена на GPU (DirectML)");
            return Ok(session);
        }
        eprintln!("[INFO] DirectML недоступен для '{model_name}'. Запуск на CPU...");
    }

    let mut builder = Session::builder().map_err(|e| anyhow::anyhow!("{e}"))?;
    builder = builder
        .with_optimization_level(optimization_level)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if let Some(threads) = intra_threads {
        builder = builder
            .with_intra_threads(threads)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }

    let session = builder
        .commit_from_memory(model_bytes)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!("[OK] Модель '{model_name}' запущена на CPU");
    Ok(session)
}
