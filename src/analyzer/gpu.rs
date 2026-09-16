use ort::session::builder::SessionBuilder;

pub fn with_gpu_providers(builder: SessionBuilder) -> anyhow::Result<SessionBuilder> {
    #[cfg(target_os = "windows")]
    {
        use ort::ep::DirectML;
        return builder
            .with_execution_providers([DirectML::default().build()])
            .map_err(|e| anyhow::anyhow!("{e}"));
    }

    #[cfg(target_os = "linux")]
    {
        use ort::ep::CUDA;
        return builder
            .with_execution_providers([CUDA::default().build()])
            .map_err(|e| anyhow::anyhow!("{e}"));
    }

    #[cfg(target_os = "macos")]
    {
        use ort::ep::CoreML;
        return builder
            .with_execution_providers([CoreML::default().build()])
            .map_err(|e| anyhow::anyhow!("{e}"));
    }

    #[allow(unreachable_code)]
    Ok(builder)
}
