fn save_gaddawg<T: gaddawg::NeedGaddawg>(
    giant_string: &str,
    output_filename: &str,
) -> gaddawg::Returns<()> {
    let t0 = std::time::Instant::now();
    let machine_words = gaddawg::read_english_machine_words(&giant_string)?;
    drop(giant_string);
    let t1 = std::time::Instant::now();
    println!(
        "{:10}ns to construct the machine words ({} words)",
        (t1 - t0).as_nanos(),
        machine_words.len()
    );
    let bin = gaddawg::build::<T>(&machine_words)?;
    drop(machine_words);
    let t2 = std::time::Instant::now();
    println!(
        "{:10}ns to make the gaddawg ({} bytes)",
        (t2 - t1).as_nanos(),
        bin.len()
    );
    std::fs::write(output_filename, bin)?;
    let t3 = std::time::Instant::now();
    println!(
        "{:10}ns to save the gaddawg into {}",
        (t3 - t2).as_nanos(),
        output_filename
    );
    Ok(())
}

fn save_gaddawg_from_file<T: gaddawg::NeedGaddawg>(
    input_filename: &str,
    output_filename: &str,
) -> gaddawg::Returns<()> {
    let t0 = std::time::Instant::now();
    // Memory wastage notes:
    // - We allocate and read the whole file at once.
    // - We could have streamed it, but that's noticeably slower.
    let giant_string = std::fs::read_to_string(input_filename)?;
    let t1 = std::time::Instant::now();
    println!(
        "{:10}ns to read the lexicon from {} ({} bytes)",
        (t1 - t0).as_nanos(),
        input_filename,
        giant_string.len()
    );
    save_gaddawg::<T>(&giant_string, output_filename)
}

fn main() -> gaddawg::Returns<()> {
    save_gaddawg_from_file::<gaddawg::DawgOnly>("leaves.txt", "leaves.gdw")?;
    save_gaddawg_from_file::<gaddawg::Gaddawg>("csw19.txt", "csw19.gdw")?;
    save_gaddawg_from_file::<gaddawg::Gaddawg>("nwl18.txt", "nwl18.gdw")?;
    save_gaddawg_from_file::<gaddawg::Gaddawg>("nwl20.txt", "nwl20.gdw")?;
    save_gaddawg::<gaddawg::Gaddawg>("VOLOST\nVOLOSTS", "volost.gdw")?;
    save_gaddawg::<gaddawg::Gaddawg>("", "empty.gdw")?;
    Ok(())
}
