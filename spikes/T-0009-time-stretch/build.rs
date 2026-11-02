fn main() {
    println!("cargo:rerun-if-changed=src/newhook.cpp");
    cc::Build::new().cpp(true).file("src/newhook.cpp").compile("newhook");
}
