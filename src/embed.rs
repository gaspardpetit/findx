/// Dimensionality of the simple embedding.
pub const EMBED_DIM: usize = 32;

/// Compute a simple normalized embedding for the given text.
///
/// This implementation hashes Unicode scalar values into a fixed-size
/// vector and normalizes the result to unit length.
pub fn embed_text(text: &str) -> Vec<f32> {
    let mut v = vec![0f32; EMBED_DIM];
    for ch in text.chars() {
        let idx = (ch as usize) % EMBED_DIM;
        v[idx] += 1.0;
    }
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}
