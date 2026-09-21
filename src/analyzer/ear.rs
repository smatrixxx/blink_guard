pub fn compute_ear_3d(eye: &[(f32, f32, f32)]) -> f32 {
    debug_assert_eq!(eye.len(), 6);
    let dist = |a: (f32, f32, f32), b: (f32, f32, f32)| {
        ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)).sqrt()
    };
    let vertical = dist(eye[1], eye[5]) + dist(eye[2], eye[4]);
    let horizontal = dist(eye[0], eye[3]);
    if horizontal < 1e-4 {
        return 0.0;
    }
    vertical / (2.0 * horizontal)
}
