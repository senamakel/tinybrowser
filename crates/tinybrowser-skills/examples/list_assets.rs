//! List every embedded skill asset and its byte size.

fn main() {
    for asset in tinybrowser_skills::skill_assets() {
        println!("{}\t{} bytes", asset.path, asset.contents.len());
    }
}
