#[derive(serde::Serialize, serde::Deserialize)]
pub struct FPCalcJsonOutput {
    pub duration: f64,
    pub fingerprint: String,
}
