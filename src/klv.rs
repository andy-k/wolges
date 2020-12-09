use super::build::MyHasherDefault;
use super::kwg;

// kwg is double-boxed :-(
// so is hashmap
pub struct Klv {
    pub kwg: Box<kwg::Kwg>,
    pub counts: Box<[u32]>,
    pub leaves: Box<[f32]>,
    pub hashmap: Box<std::collections::HashMap<Box<[u8]>, f32, MyHasherDefault>>,
}

impl Klv {
    pub fn from_bytes_alloc(buf: &[u8]) -> Klv {
        let mut r = 0;
        let kwg_bytes_len = (u32::from_le(
            buf[r] as u32
                | (buf[r + 1] as u32) << 8
                | (buf[r + 2] as u32) << 16
                | (buf[r + 3] as u32) << 24,
        ) as usize)
            * 4;
        r += 4;
        let kwg = kwg::Kwg::from_bytes_alloc(&buf[r..r + kwg_bytes_len]);
        r += kwg_bytes_len;
        let lv_len = u32::from_le(
            buf[r] as u32
                | (buf[r + 1] as u32) << 8
                | (buf[r + 2] as u32) << 16
                | (buf[r + 3] as u32) << 24,
        );
        r += 4;
        let mut elts = Vec::with_capacity(lv_len as usize);
        for _ in 0..lv_len {
            elts.push(f32::from_bits(u32::from_le(
                buf[r] as u32
                    | (buf[r + 1] as u32) << 8
                    | (buf[r + 2] as u32) << 16
                    | (buf[r + 3] as u32) << 24,
            )));
            r += 4;
        }
        let counts = kwg.count_words_alloc();
        let mut hashmap =
            std::collections::HashMap::<_, _, MyHasherDefault>::with_capacity_and_hasher(
                counts[0] as usize,
                MyHasherDefault::default(),
            );

        struct Env<'a> {
            g: &'a kwg::Kwg,
            s: &'a mut Vec<u8>,
            it: std::slice::Iter<'a, f32>,
            hashmap: &'a mut std::collections::HashMap<Box<[u8]>, f32, MyHasherDefault>,
        }
        fn iter(env: &mut Env, mut p: i32) {
            loop {
                let t = env.g[p].tile();
                env.s.push(t);
                if env.g[p].accepts() {
                    env.hashmap
                        .insert(env.s.clone().into_boxed_slice(), *env.it.next().unwrap());
                }
                if env.g[p].arc_index() != 0 {
                    iter(env, env.g[p].arc_index());
                }
                env.s.pop();
                if env.g[p].is_end() {
                    break;
                }
                p += 1;
            }
        }
        iter(
            &mut Env {
                g: &kwg,
                s: &mut Vec::new(),
                it: elts.iter(),
                hashmap: &mut hashmap,
            },
            kwg[0i32].arc_index(),
        );

        // sth similar to prt_dawg
        Klv {
            kwg: Box::new(kwg),
            counts,
            leaves: elts.into_boxed_slice(),
            hashmap: Box::new(hashmap),
        }
    }
}
