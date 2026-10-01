// Copyright (C) 2020-2026 Andy Kurnia.

pub struct WordList {
    blob: Vec<u8>,
    ends: Vec<u32>,
}

impl WordList {
    #[inline(always)]
    pub fn build<N: super::kwg::Node>(g: &super::kwg::Kwg<N>) -> Self {
        fn walk<N: super::kwg::Node>(
            g: &super::kwg::Kwg<N>,
            mut p: i32,
            w: &mut Vec<u8>,
            out: &mut WordList,
        ) {
            if p <= 0 {
                return;
            }
            loop {
                let node = g[p];
                w.push(node.tile());
                if node.accepts() {
                    out.blob.extend_from_slice(w);
                    out.ends.push(out.blob.len() as u32);
                }
                walk(g, node.arc_index(), w, out);
                w.pop();
                if node.is_end() {
                    return;
                }
                p += 1;
            }
        }
        let mut out = WordList {
            blob: Vec::new(),
            ends: Vec::new(),
        };
        walk(g, g[0].arc_index(), &mut Vec::new(), &mut out);
        out
    }

    pub fn iter(&self) -> impl Iterator<Item = &[u8]> {
        let mut at = 0usize;
        self.ends.iter().map(move |&end| {
            let w = &self.blob[at..end as usize];
            at = end as usize;
            w
        })
    }
}

pub struct Words<'a> {
    pub words: &'a [u8],
    pub len: u8,
}

impl<'a> Words<'a> {
    #[inline(always)]
    pub fn iter(&self) -> impl Iterator<Item = &'a [u8]> {
        self.words.chunks_exact(self.len as usize)
    }
}
