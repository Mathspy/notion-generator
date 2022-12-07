use std::iter::{self, Map};

const COOL: &str = "abc";

#[derive(Debug)]
struct Abc {
    field: i32,
}

/// Doc comment!
enum Xyz {
    XVariant { field: u32 },
    YVariant(f32),
    ZVariant,
}

#[some_attr_macro]
fn other_fn<'a, T>(
    arg1: &'a mut T,
    arg2: String,
    arg3: &'static str,
) -> impl Iterator<Item = String>
where
    T: Debug,
{
}

// This is the main function
fn main() {
    // Statements here are executed when the compiled binary is called
    // Print text to the console
    println!("Hello World!");

    let logical: bool = true || false && true;
    let a_float: f64 = 1.0 + 2.0 * 3.0; // Regular annotation
    let mut integer = 5i32 as f32;
    let mut boolean: bool = a_float as i32 > 5;

    let (x, y, z) = ([1, 2, 3], [4, 5], [6]);

    match x {
        [1, ..] => {
            println!("{}", 1);
        }
        [2 | 3, ..] => {}
        [4, x, y] if x == y => {}
        n @ [10, ..] => {}
        _ => {}
    };

    if logical {
        for something in x {
            loop {
                break;
            }
        }
    }

    (1..10).map(|x| x * 3).collect::<Vec<_>>();

    match Xyz {
        XVariant { field } => {}
        YVariant(whatever) => {}
        ZVariant => {}
        fallback => {}
    };
}

macro_rules! print_result {
    ($expression:expr) => {
        println!("{:?} = {:?}", stringify!($expression), $expression);
    };
}

#[cfg(test)]
mod tests {
    use super::other_fn;

    #[test]
    fn welp() {}
}
