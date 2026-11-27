use super::sun_fits;

#[test]
fn sun_fits_at_the_boundary() {
    // A 100 px label needs 100 + 8 + 14 = 122 px.
    assert!(sun_fits(100., 122.));
    assert!(!sun_fits(100., 121.9));
    assert!(sun_fits(100., 200.));
    assert!(!sun_fits(0., 21.9));
    assert!(sun_fits(0., 22.));
}
