fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        serde_json::to_string_pretty(&horizon_client_lib::yoake_state_catalog::catalog())?
    );
    Ok(())
}
