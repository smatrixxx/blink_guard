use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;

pub fn create_session_from_bytes(
    model_bytes: &[u8],
    model_name: &str,
    optimization_level: GraphOptimizationLevel,
    intra_threads: Option<usize>,
) -> anyhow::Result<Session> {
    // 1. Попытка на GPU
    let gpu_result = (|| -> ort::Result<Session> {
        let mut builder = Session::builder()?;

        #[cfg(target_os = "windows")]
        {
            use ort::ep::DirectML;
            builder = builder.with_execution_providers([DirectML::default().build()])?;
        }

        #[cfg(target_os = "linux")]
        {
            use ort::ep::CUDA;
            builder = builder.with_execution_providers([CUDA::default().build()])?;
        }

        #[cfg(target_os = "macos")]
        {
            use ort::ep::CoreML;
            builder = builder.with_execution_providers([CoreML::default().build()])?;
        }

        builder = builder.with_optimization_level(optimization_level)?;
        if let Some(threads) = intra_threads {
            builder = builder.with_intra_threads(threads)?;
        }

        builder.commit_from_memory(model_bytes)
    })();

    match gpu_result {
        Ok(session) => {
            println!("✓ Модель '{model_name}' запущена на GPU");
            Ok(session)
        }
        Err(err) => {
            eprintln!("⚠ Не удалось запустить '{model_name}' на GPU ({err}). Запуск на CPU...");

            // 2. Чистый Fallback на CPU
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

            println!("✓ Модель '{model_name}' запущена на CPU");
            Ok(session)
        }
    }
}
