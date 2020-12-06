pub struct Node(pub u32);

impl Node {
    #[inline(always)]
    fn tile(&self) -> u8 {
        (self.0 >> 24) as u8
    }

    #[inline(always)]
    fn accepts(&self) -> bool {
        self.0 & 0x800000 != 0
    }

    #[inline(always)]
    fn is_end(&self) -> bool {
        self.0 & 0x400000 != 0
    }

    #[inline(always)]
    fn arc_index(&self) -> u32 {
        self.0 & 0x3fffff
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

impl std::ops::Index<u32> for Gdw {
    type Output = Node;

    #[inline(always)]
    fn index(&self, i: u32) -> &Node {
        &self[i as usize]
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
