#![no_main]

ziskos::entrypoint!(main);

#[cfg(not(feature = "accelerated"))]
#[path = "reference.rs"]
mod phgr;

#[cfg(feature = "accelerated")]
#[path = "accelerated.rs"]
mod phgr;

fn main() {
    let input = ziskos::io::read_slice();
    assert_eq!(input.len(), 568);
    let accepted = phgr::verify(&input[..296], &input[296..]).is_ok();
    ziskos::io::commit_slice(&[0x5a, u8::from(accepted), 0xa5]);
}
