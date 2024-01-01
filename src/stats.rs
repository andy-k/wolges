// Copyright (C) 2020-2024 Andy Kurnia.

pub struct Stats {
    count: f64, // should be a non-negative int barring overflows
    mean: f64,
    m2: f64,
}

impl Default for Stats {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

impl Stats {
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            count: 0.0,
            mean: 0.0,
            m2: 0.0,
        }
    }

    // https://en.wikipedia.org/wiki/Algorithms_for_calculating_variance#Welford's_online_algorithm
    #[inline(always)]
    pub fn update(&mut self, new_value: f64) {
        self.count += 1.0;
        let delta = new_value - self.mean;
        self.mean += delta / self.count;
        let delta2 = new_value - self.mean;
        self.m2 += delta * delta2;
    }

    // https://en.wikipedia.org/wiki/Algorithms_for_calculating_variance#Parallel_algorithm
    #[allow(dead_code)]
    #[inline(always)]
    pub fn update_bulk(&mut self, other: &Stats) {
        let original_count = self.count;
        self.count += other.count;
        if self.count != 0.0 {
            // this branch is predictable
            let delta = other.mean - self.mean;
            let delta_mean = delta * (other.count / self.count);
            self.mean += delta_mean;
            self.m2 += other.m2 + delta * delta_mean * original_count;
        }
    }

    #[allow(dead_code)]
    #[inline(always)]
    pub fn count(&self) -> f64 {
        self.count
    }

    #[inline(always)]
    pub fn mean(&self) -> f64 {
        self.mean
    }

    #[inline(always)]
    pub fn variance(&self) -> f64 {
        // this branch is largely predictable
        if self.count < 2.0 {
            0.0
        } else {
            self.m2 / (self.count - 1.0)
        }
    }

    #[allow(dead_code)]
    #[inline(always)]
    pub fn standard_deviation(&self) -> f64 {
        self.variance().sqrt()
    }

    // https://www.mathsisfun.com/data/confidence-interval.html
    #[inline(always)]
    pub fn ci_max(&self, z: f64) -> f64 {
        self.mean + z * (self.variance() / self.count).sqrt()
    }
}
