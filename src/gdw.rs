pub struct Node(pub u32);

impl Node {
    #[inline(always)]
    pub fn tile(&self) -> u8 {
        (self.0 >> 24) as u8
    }

    #[inline(always)]
    pub fn accepts(&self) -> bool {
        self.0 & 0x800000 != 0
    }

    #[inline(always)]
    pub fn is_end(&self) -> bool {
        self.0 & 0x400000 != 0
    }

    #[inline(always)]
    pub fn arc_index(&self) -> i32 {
        (self.0 & 0x3fffff) as i32
    }
}

pub struct Gdw(pub Box<[Node]>);

impl std::ops::Index<usize> for Gdw {
    type Output = Node;

    #[inline(always)]
    fn index(&self, i: usize) -> &Node {
        &self.0[i]
    }
}

impl std::ops::Index<i32> for Gdw {
    type Output = Node;

    #[inline(always)]
    fn index(&self, i: i32) -> &Node {
        &self[i as usize]
    }
}

impl Gdw {
    pub fn in_gdw(&self, mut p: i32, tile: u8) -> i32 {
        if p >= 0 {
            p = self[p].arc_index() as i32;
            if p > 0 {
                while self[p].tile() != tile {
                    if self[p].is_end() {
                        return -1;
                    }
                    p += 1;
                }
                return p;
            }
        }
        -1 // intentionally return 0 as -1
    }
}

pub fn from_bytes(buf: &[u8]) -> Gdw {
    let nelts = buf.len() / 4;
    let mut elts = Vec::with_capacity(nelts);
    for r in (0..(nelts * 4)).step_by(4) {
        elts.push(Node(u32::from_le(
            buf[r] as u32
                | (buf[r + 1] as u32) << 8
                | (buf[r + 2] as u32) << 16
                | (buf[r + 3] as u32) << 24,
        )));
    }
    Gdw(elts.into_boxed_slice())
}
