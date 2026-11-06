// ArrayTimSort from Node 24.21.0 / V8 13.6.233.17-node.53.
// Copyright (c) 2001-2017 Python Software Foundation; All Rights Reserved.
// See vendor/v8 for the original source, provenance and licenses.
// The comparison schedule matters for upstream comparators that are not transitive.
use std::cmp::Ordering;

pub fn sort_by<T>(values: &mut [T], mut compare: impl FnMut(&T, &T) -> Ordering) {
    if values.len() < 2 {
        return;
    }
    let mut state = SortState {
        work: (0..values.len()).collect(),
        runs: vec![],
        min_gallop: 7,
        compare: |left, right| compare(&values[left], &values[right]),
    };
    state.sort();
    let mut destinations = vec![0; values.len()];
    for (destination, source) in state.work.into_iter().enumerate() {
        destinations[source] = destination;
    }
    for index in 0..values.len() {
        while destinations[index] != index {
            let destination = destinations[index];
            values.swap(index, destination);
            destinations.swap(index, destination);
        }
    }
}

struct SortState<F> {
    work: Vec<usize>,
    runs: Vec<(usize, usize)>,
    min_gallop: usize,
    compare: F,
}

impl<F: FnMut(usize, usize) -> Ordering> SortState<F> {
    fn sort(&mut self) {
        let mut n = self.work.len();
        let mut remainder = 0;
        while n >= 64 {
            remainder |= n & 1;
            n >>= 1;
        }
        let min_run = n + remainder;
        let mut low = 0;
        while low < self.work.len() {
            let mut length = self.count_run(low);
            if length < min_run {
                let forced = min_run.min(self.work.len() - low);
                self.binary_insertion(low, low + length, low + forced);
                length = forced;
            }
            self.runs.push((low, length));
            self.collapse();
            low += length;
        }
        while self.runs.len() > 1 {
            let mut n = self.runs.len() - 2;
            if n > 0 && self.runs[n - 1].1 < self.runs[n + 1].1 {
                n -= 1;
            }
            self.merge_at(n);
        }
    }

    fn count_run(&mut self, low: usize) -> usize {
        if low + 1 == self.work.len() {
            return 1;
        }
        let descending = (self.compare)(self.work[low + 1], self.work[low]) == Ordering::Less;
        let mut length = 2;
        while low + length < self.work.len() {
            let order = (self.compare)(self.work[low + length], self.work[low + length - 1]);
            if (order == Ordering::Less) != descending {
                break;
            }
            length += 1;
        }
        if descending {
            self.work[low..low + length].reverse();
        }
        length
    }

    fn binary_insertion(&mut self, low: usize, start: usize, high: usize) {
        for start in start.max(low + 1)..high {
            let pivot = self.work[start];
            let (mut left, mut right) = (low, start);
            while left < right {
                let middle = left + (right - left) / 2;
                if (self.compare)(pivot, self.work[middle]) == Ordering::Less {
                    right = middle;
                } else {
                    left = middle + 1;
                }
            }
            self.work.copy_within(left..start, left + 1);
            self.work[left] = pivot;
        }
    }

    fn invariant(&self, n: usize) -> bool {
        n < 2 || self.runs[n - 2].1 > self.runs[n - 1].1 + self.runs[n].1
    }

    fn collapse(&mut self) {
        while self.runs.len() > 1 {
            let mut n = self.runs.len() - 2;
            if !self.invariant(n + 1) || !self.invariant(n) {
                if self.runs[n - 1].1 < self.runs[n + 1].1 {
                    n -= 1;
                }
                self.merge_at(n);
            } else if self.runs[n].1 <= self.runs[n + 1].1 {
                self.merge_at(n);
            } else {
                break;
            }
        }
    }

    fn merge_at(&mut self, index: usize) {
        let (mut base_a, mut length_a) = self.runs[index];
        let (base_b, mut length_b) = self.runs.remove(index + 1);
        self.runs[index].1 += length_b;
        let skip = gallop_right(
            &self.work,
            self.work[base_b],
            base_a,
            length_a,
            0,
            &mut self.compare,
        );
        base_a += skip;
        length_a -= skip;
        if length_a == 0 {
            return;
        }
        length_b = gallop_left(
            &self.work,
            self.work[base_a + length_a - 1],
            base_b,
            length_b,
            length_b - 1,
            &mut self.compare,
        );
        if length_b == 0 {
            return;
        }
        if length_a <= length_b {
            self.merge_low(base_a, length_a, base_b, length_b);
        } else {
            self.merge_high(base_a, length_a, base_b, length_b);
        }
    }

    fn merge_low(
        &mut self,
        base_a: usize,
        mut length_a: usize,
        base_b: usize,
        mut length_b: usize,
    ) {
        let temporary = self.work[base_a..base_a + length_a].to_vec();
        let (mut destination, mut cursor_a, mut cursor_b) = (base_a, 0, base_b);
        self.work[destination] = self.work[cursor_b];
        destination += 1;
        cursor_b += 1;
        length_b -= 1;
        let mut min_gallop = self.min_gallop;
        'merge: while length_b > 0 && length_a > 1 {
            let (mut wins_a, mut wins_b) = (0, 0);
            loop {
                if (self.compare)(self.work[cursor_b], temporary[cursor_a]) == Ordering::Less {
                    self.work[destination] = self.work[cursor_b];
                    destination += 1;
                    cursor_b += 1;
                    length_b -= 1;
                    wins_b += 1;
                    wins_a = 0;
                    if length_b == 0 {
                        break 'merge;
                    }
                    if wins_b >= min_gallop {
                        break;
                    }
                } else {
                    self.work[destination] = temporary[cursor_a];
                    destination += 1;
                    cursor_a += 1;
                    length_a -= 1;
                    wins_a += 1;
                    wins_b = 0;
                    if length_a == 1 {
                        break 'merge;
                    }
                    if wins_a >= min_gallop {
                        break;
                    }
                }
            }
            min_gallop += 1;
            let mut first = true;
            while wins_a >= 7 || wins_b >= 7 || first {
                first = false;
                min_gallop = 1.max(min_gallop - 1);
                self.min_gallop = min_gallop;
                wins_a = gallop_right(
                    &temporary,
                    self.work[cursor_b],
                    cursor_a,
                    length_a,
                    0,
                    &mut self.compare,
                );
                if wins_a > 0 {
                    self.work[destination..destination + wins_a]
                        .copy_from_slice(&temporary[cursor_a..cursor_a + wins_a]);
                    destination += wins_a;
                    cursor_a += wins_a;
                    length_a -= wins_a;
                    if length_a <= 1 {
                        break 'merge;
                    }
                }
                self.work[destination] = self.work[cursor_b];
                destination += 1;
                cursor_b += 1;
                length_b -= 1;
                if length_b == 0 {
                    break 'merge;
                }
                wins_b = gallop_left(
                    &self.work,
                    temporary[cursor_a],
                    cursor_b,
                    length_b,
                    0,
                    &mut self.compare,
                );
                if wins_b > 0 {
                    self.work
                        .copy_within(cursor_b..cursor_b + wins_b, destination);
                    destination += wins_b;
                    cursor_b += wins_b;
                    length_b -= wins_b;
                    if length_b == 0 {
                        break 'merge;
                    }
                }
                self.work[destination] = temporary[cursor_a];
                destination += 1;
                cursor_a += 1;
                length_a -= 1;
                if length_a == 1 {
                    break 'merge;
                }
            }
            min_gallop += 1;
            self.min_gallop = min_gallop;
        }
        if length_a == 1 && length_b > 0 {
            self.work
                .copy_within(cursor_b..cursor_b + length_b, destination);
            self.work[destination + length_b] = temporary[cursor_a];
        } else if length_a > 0 {
            self.work[destination..destination + length_a]
                .copy_from_slice(&temporary[cursor_a..cursor_a + length_a]);
        }
    }

    fn merge_high(
        &mut self,
        base_a: usize,
        mut length_a: usize,
        base_b: usize,
        mut length_b: usize,
    ) {
        let temporary = self.work[base_b..base_b + length_b].to_vec();
        // Exclusive cursors avoid negative offsets after copying the first item.
        let (mut destination, mut cursor_b, mut cursor_a) =
            (base_b + length_b, length_b, base_a + length_a);
        destination -= 1;
        cursor_a -= 1;
        self.work[destination] = self.work[cursor_a];
        length_a -= 1;
        let mut min_gallop = self.min_gallop;
        'merge: while length_a > 0 && length_b > 1 {
            let (mut wins_a, mut wins_b) = (0, 0);
            loop {
                if (self.compare)(temporary[cursor_b - 1], self.work[cursor_a - 1])
                    == Ordering::Less
                {
                    destination -= 1;
                    cursor_a -= 1;
                    self.work[destination] = self.work[cursor_a];
                    length_a -= 1;
                    wins_a += 1;
                    wins_b = 0;
                    if length_a == 0 {
                        break 'merge;
                    }
                    if wins_a >= min_gallop {
                        break;
                    }
                } else {
                    destination -= 1;
                    cursor_b -= 1;
                    self.work[destination] = temporary[cursor_b];
                    length_b -= 1;
                    wins_b += 1;
                    wins_a = 0;
                    if length_b == 1 {
                        break 'merge;
                    }
                    if wins_b >= min_gallop {
                        break;
                    }
                }
            }
            min_gallop += 1;
            let mut first = true;
            while wins_a >= 7 || wins_b >= 7 || first {
                first = false;
                min_gallop = 1.max(min_gallop - 1);
                self.min_gallop = min_gallop;
                let k = gallop_right(
                    &self.work,
                    temporary[cursor_b - 1],
                    base_a,
                    length_a,
                    length_a - 1,
                    &mut self.compare,
                );
                wins_a = length_a - k;
                if wins_a > 0 {
                    destination -= wins_a;
                    cursor_a -= wins_a;
                    self.work
                        .copy_within(cursor_a..cursor_a + wins_a, destination);
                    length_a -= wins_a;
                    if length_a == 0 {
                        break 'merge;
                    }
                }
                destination -= 1;
                cursor_b -= 1;
                self.work[destination] = temporary[cursor_b];
                length_b -= 1;
                if length_b == 1 {
                    break 'merge;
                }
                let k = gallop_left(
                    &temporary,
                    self.work[cursor_a - 1],
                    0,
                    length_b,
                    length_b - 1,
                    &mut self.compare,
                );
                wins_b = length_b - k;
                if wins_b > 0 {
                    destination -= wins_b;
                    cursor_b -= wins_b;
                    self.work[destination..destination + wins_b]
                        .copy_from_slice(&temporary[cursor_b..cursor_b + wins_b]);
                    length_b -= wins_b;
                    if length_b <= 1 {
                        break 'merge;
                    }
                }
                destination -= 1;
                cursor_a -= 1;
                self.work[destination] = self.work[cursor_a];
                length_a -= 1;
                if length_a == 0 {
                    break 'merge;
                }
            }
            min_gallop += 1;
            self.min_gallop = min_gallop;
        }
        if length_b == 1 && length_a > 0 {
            destination -= length_a;
            cursor_a -= length_a;
            self.work
                .copy_within(cursor_a..cursor_a + length_a, destination);
            self.work[destination - 1] = temporary[cursor_b - 1];
        } else if length_b > 0 {
            self.work[destination - length_b..destination].copy_from_slice(&temporary[..length_b]);
        }
    }
}

fn gallop_left(
    array: &[usize],
    key: usize,
    base: usize,
    length: usize,
    hint: usize,
    compare: &mut impl FnMut(usize, usize) -> Ordering,
) -> usize {
    let (mut last, mut offset) = (0_usize, 1_usize);
    let (mut low, mut high);
    if compare(array[base + hint], key) == Ordering::Less {
        let maximum = length - hint;
        while offset < maximum && compare(array[base + hint + offset], key) == Ordering::Less {
            last = offset;
            offset = offset.saturating_mul(2).saturating_add(1);
        }
        low = hint + last + 1;
        high = hint + offset.min(maximum);
    } else {
        let maximum = hint + 1;
        while offset < maximum && compare(array[base + hint - offset], key) != Ordering::Less {
            last = offset;
            offset = offset.saturating_mul(2).saturating_add(1);
        }
        low = hint + 1 - offset.min(maximum);
        high = hint - last;
    }
    while low < high {
        let middle = low + (high - low) / 2;
        if compare(array[base + middle], key) == Ordering::Less {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    high
}

fn gallop_right(
    array: &[usize],
    key: usize,
    base: usize,
    length: usize,
    hint: usize,
    compare: &mut impl FnMut(usize, usize) -> Ordering,
) -> usize {
    let (mut last, mut offset) = (0_usize, 1_usize);
    let (mut low, mut high);
    if compare(key, array[base + hint]) == Ordering::Less {
        let maximum = hint + 1;
        while offset < maximum && compare(key, array[base + hint - offset]) == Ordering::Less {
            last = offset;
            offset = offset.saturating_mul(2).saturating_add(1);
        }
        low = hint + 1 - offset.min(maximum);
        high = hint - last;
    } else {
        let maximum = length - hint;
        while offset < maximum && compare(key, array[base + hint + offset]) != Ordering::Less {
            last = offset;
            offset = offset.saturating_mul(2).saturating_add(1);
        }
        low = hint + last + 1;
        high = hint + offset.min(maximum);
    }
    while low < high {
        let middle = low + (high - low) / 2;
        if compare(key, array[base + middle]) == Ordering::Less {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    high
}
