mod gdw;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gdw = gdw::from_bytes(&std::fs::read("csw19.gdw")?);
    println!("{}", gdw.0.len());
    println!("Hello, world!");
    Ok(())
}
