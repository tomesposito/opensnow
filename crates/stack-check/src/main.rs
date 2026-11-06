async fn check() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    opensnow_engine::check().await?;
    opensnow_storage::check().await?;
    Ok(())
}

#[tokio::main]
async fn main() {
    std::panic::set_hook(Box::new(|info| eprintln!("FAIL stack-check: {info}")));
    if let Err(error) = check().await {
        eprintln!("FAIL stack-check: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn combined_stack() {
        super::check().await.expect("both paths must pass in one process");
    }
}
