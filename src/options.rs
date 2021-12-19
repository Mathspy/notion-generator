#[derive(Clone, Copy)]
pub enum HeadingAnchors<'a> {
    None,
    Before(&'a str),
    After(&'a str),
}
