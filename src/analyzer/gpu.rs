use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;

pub fn create_session_with_fallback(
    model_path: &str,
    optimization_level: GraphOptimizationLevel,
    intra_threads: Option<usize>,
) -> anyhow::Result<Session> {
    // 1. Пытаемся запустить с GPU
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

        builder.commit_from_file(model_path)
    })();

    match gpu_result {
        Ok(session) => {
            println!("✓ Модель '{model_path}' успешно запущена на GPU");
            Ok(session)
        }
        Err(err) => {
            eprintln!(
                "⚠ Не удалось запустить '{model_path}' на GPU ({err}). Выполняем fallback на CPU..."
            );

            // 2. Чистый fallback на CPU при любой ошибке GPU
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
                .commit_from_file(model_path)
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            println!("✓ Модель '{model_path}' успешно запущена на CPU");
            Ok(session)
        }
    }
}
