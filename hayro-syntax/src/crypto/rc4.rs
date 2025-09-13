#[derive(Clone)]
pub(crate) struct Rc4 {
    a: u8,
    b: u8,
    s: [u8; 256],
}

impl Rc4 {
    pub(crate) fn new(key: &[u8]) -> Self {
        let mut s = [0u8; 256];
        let key_length = key.len();

        for i in 0..256 {
            s[i] = i as u8;
        }

        let mut j = 0u8;
        for i in 0..256 {
            let tmp = s[i];
            j = j.wrapping_add(tmp).wrapping_add(key[i % key_length]);
            s[i] = s[j as usize];
            s[j as usize] = tmp;
        }

        Rc4 { a: 0, b: 0, s }
    }

    pub(crate) fn decrypt(&mut self, data: &[u8]) -> Vec<u8> {
        let n = data.len();
        let mut output = vec![0u8; n];

        for i in 0..n {
            self.a = self.a.wrapping_add(1);
            let tmp = self.s[self.a as usize];
            self.b = self.b.wrapping_add(tmp);
            let tmp2 = self.s[self.b as usize];
            self.s[self.a as usize] = tmp2;
            self.s[self.b as usize] = tmp;
            output[i] = data[i] ^ self.s[tmp.wrapping_add(tmp2) as usize];
        }

        output
    }
}
