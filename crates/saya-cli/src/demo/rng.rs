pub(crate) struct Lcg(u64);

impl Lcg {
    pub(crate) const fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn step(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }

    pub(crate) fn below(&mut self, bound: u64) -> u64 {
        self.step() % bound
    }
}
