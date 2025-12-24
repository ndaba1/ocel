use rand::{Rng, rng};

pub fn get_nanoid(length: usize) -> String {
    const CHARSET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut r = rng();

    (0..length)
        .map(|_| {
            let idx = r.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}
