fn main() {
    [1, 2, 3].into_iter().collect::<Vec<_>>();
    Err::<Option<usize>, ()>(());
    <Option<()>>::map(Some(()), |_| {});

    // turbospider, a rare species
    Result::<(), _>::Err(());
}
