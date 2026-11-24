//! The digests EncodingService:ComputeBufferHash and ComputeStringHash answer, one for each HashAlgorithm: MD5 (RFC 1321), SHA-1 and
//! SHA-256 (FIPS 180-4), BLAKE2b (RFC 7693, unkeyed, any digest length) and BLAKE3 (its hash mode, the 32-byte default output).
//! Written here so that every build has them, a WebAssembly one included: the sha2 crate comes only with the `bundles` feature. Each is
//! checked against its published test vectors (the tests below) and against a playing client's answers
//! (tools/parity/probes/encoding_hash.luau).

/// Hands `input` to `block` in 64-byte blocks with the padding MD5, SHA-1 and SHA-256 share: a 1 bit, zeros, and the input's length in
/// bits as 8 bytes, little-endian for MD5 and big-endian (`big`) for SHA.
fn padded_blocks(input: &[u8], big: bool, mut block: impl FnMut(&[u8; 64])) {
    let (blocks, rest) = input.as_chunks::<64>();
    for chunk in blocks {
        block(chunk);
    }
    let bits = (input.len() as u64).wrapping_mul(8);
    let mut tail = [0u8; 128];
    tail[..rest.len()].copy_from_slice(rest);
    tail[rest.len()] = 0x80;
    let end = if rest.len() < 56 { 64 } else { 128 };
    tail[end - 8..end].copy_from_slice(&if big { bits.to_be_bytes() } else { bits.to_le_bytes() });
    for chunk in tail[..end].as_chunks::<64>().0 {
        block(chunk);
    }
}

fn word_le(bytes: &[u8], index: usize) -> u32 {
    u32::from_le_bytes([bytes[4 * index], bytes[4 * index + 1], bytes[4 * index + 2], bytes[4 * index + 3]])
}

fn word_be(bytes: &[u8], index: usize) -> u32 {
    u32::from_be_bytes([bytes[4 * index], bytes[4 * index + 1], bytes[4 * index + 2], bytes[4 * index + 3]])
}

/// The words as bytes, each little-endian or big-endian (`big`)
fn word_bytes<const N: usize>(words: &[u32], big: bool) -> [u8; N] {
    let mut out = [0u8; N];
    for (bytes, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(words) {
        bytes.copy_from_slice(&if big { word.to_be_bytes() } else { word.to_le_bytes() });
    }
    out
}

/// MD5's sines: the whole part of |sin(i + 1)| * 2^32
const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be,
    0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
    0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1, 0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1,
    0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];
/// MD5's rotations, four for each of its four rounds
const MD5_SHIFTS: [u32; 16] = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];

pub fn md5(input: &[u8]) -> [u8; 16] {
    let mut state: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    padded_blocks(input, false, |block| {
        let [mut a, mut b, mut c, mut d] = state;
        for step in 0..64 {
            let (mixed, word) = match step / 16 {
                0 => ((b & c) | (!b & d), step),
                1 => ((d & b) | (!d & c), (5 * step + 1) % 16),
                2 => (b ^ c ^ d, (3 * step + 5) % 16),
                _ => (c ^ (b | !d), (7 * step) % 16),
            };
            let sum = mixed.wrapping_add(a).wrapping_add(MD5_K[step]).wrapping_add(word_le(block, word));
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(sum.rotate_left(MD5_SHIFTS[step / 16 * 4 + step % 4]));
        }
        for (word, add) in state.iter_mut().zip([a, b, c, d]) {
            *word = word.wrapping_add(add);
        }
    });
    word_bytes(&state, false)
}

pub fn sha1(input: &[u8]) -> [u8; 20] {
    let mut state: [u32; 5] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0];
    padded_blocks(input, true, |block| {
        let mut schedule = [0u32; 80];
        for (index, word) in schedule.iter_mut().take(16).enumerate() {
            *word = word_be(block, index);
        }
        for index in 16..80 {
            schedule[index] = (schedule[index - 3] ^ schedule[index - 8] ^ schedule[index - 14] ^ schedule[index - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (step, &word) in schedule.iter().enumerate() {
            let (mixed, constant) = match step / 20 {
                0 => ((b & c) | (!b & d), 0x5a827999),
                1 => (b ^ c ^ d, 0x6ed9eba1),
                2 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let sum = a.rotate_left(5).wrapping_add(mixed).wrapping_add(e).wrapping_add(constant).wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = sum;
        }
        for (word, add) in state.iter_mut().zip([a, b, c, d, e]) {
            *word = word.wrapping_add(add);
        }
    });
    word_bytes(&state, true)
}

/// SHA-256's first state, which BLAKE3 starts from too: the first 32 bits of the fractional parts of the square roots of the first
/// eight primes
const SHA256_IV: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
/// SHA-256's round constants: the first 32 bits of the fractional parts of the cube roots of the first 64 primes
const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256(input: &[u8]) -> [u8; 32] {
    let mut state = SHA256_IV;
    padded_blocks(input, true, |block| {
        let mut schedule = [0u32; 64];
        for (index, word) in schedule.iter_mut().take(16).enumerate() {
            *word = word_be(block, index);
        }
        for index in 16..64 {
            let (early, late) = (schedule[index - 15], schedule[index - 2]);
            let small0 = early.rotate_right(7) ^ early.rotate_right(18) ^ (early >> 3);
            let small1 = late.rotate_right(17) ^ late.rotate_right(19) ^ (late >> 10);
            schedule[index] = schedule[index - 16].wrapping_add(small0).wrapping_add(schedule[index - 7]).wrapping_add(small1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for (&word, &constant) in schedule.iter().zip(&SHA256_K) {
            let big1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ (!e & g);
            let first = h.wrapping_add(big1).wrapping_add(choose).wrapping_add(constant).wrapping_add(word);
            let big0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let second = big0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(first);
            d = c;
            c = b;
            b = a;
            a = first.wrapping_add(second);
        }
        for (word, add) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *word = word.wrapping_add(add);
        }
    });
    word_bytes(&state, true)
}

/// BLAKE2b's first state (SHA-512's)
const BLAKE2B_IV: [u64; 8] = [
    0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1, 0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b,
    0x5be0cd19137e2179,
];
/// The message words each round of BLAKE2b mixes (rounds 10 and 11 take rows 0 and 1 again); BLAKE3 permutes its own instead
const BLAKE2_SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

/// BLAKE2b's G: mixes words a, b, c and d of the work vector with two message words
fn blake2b_mix(v: &mut [u64; 16], [a, b, c, d]: [usize; 4], x: u64, y: u64) {
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
    v[d] = (v[d] ^ v[a]).rotate_right(32);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(24);
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
    v[d] = (v[d] ^ v[a]).rotate_right(16);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(63);
}

/// The columns, then the diagonals, of the 4 x 4 work vector BLAKE2b and BLAKE3 mix in each round
const MIXES: [[usize; 4]; 8] = [[0, 4, 8, 12], [1, 5, 9, 13], [2, 6, 10, 14], [3, 7, 11, 15], [0, 5, 10, 15], [1, 6, 11, 12], [2, 7, 8, 13], [3, 4, 9, 14]];

/// BLAKE2b's F: one 128-byte block into the state, `counter` the bytes hashed so far with this block's, `last` for the final block
fn blake2b_compress(state: &mut [u64; 8], block: &[u8; 128], counter: u128, last: bool) {
    let message: [u64; 16] = std::array::from_fn(|index| u64::from_le_bytes(block[8 * index..8 * index + 8].try_into().expect("8 bytes")));
    let mut v = [0u64; 16];
    v[..8].copy_from_slice(state);
    v[8..].copy_from_slice(&BLAKE2B_IV);
    v[12] ^= counter as u64;
    v[13] ^= (counter >> 64) as u64;
    if last {
        v[14] = !v[14];
    }
    for round in 0..12 {
        let sigma = &BLAKE2_SIGMA[round % 10];
        for (index, &words) in MIXES.iter().enumerate() {
            blake2b_mix(&mut v, words, message[sigma[2 * index]], message[sigma[2 * index + 1]]);
        }
    }
    for (index, word) in state.iter_mut().enumerate() {
        *word ^= v[index] ^ v[index + 8];
    }
}

/// Unkeyed BLAKE2b with a digest of `length` bytes (1 to 64): the length is part of the parameter block, so a 32-byte digest is not
/// the 64-byte one cut short.
pub fn blake2b(input: &[u8], length: usize) -> Vec<u8> {
    assert!((1..=64).contains(&length), "a BLAKE2b digest is 1 to 64 bytes");
    let mut state = BLAKE2B_IV;
    // (digest length, key length 0, fanout 1, depth 1)
    state[0] ^= 0x0101_0000 ^ length as u64;
    // every block but the last is compressed as it comes; the last (an empty input's one empty block) as the final one
    let full = input.len().saturating_sub(1) / 128;
    let mut counter: u128 = 0;
    for block in input[..128 * full].as_chunks::<128>().0 {
        counter += 128;
        blake2b_compress(&mut state, block, counter, false);
    }
    let rest = &input[128 * full..];
    counter += rest.len() as u128;
    let mut block = [0u8; 128];
    block[..rest.len()].copy_from_slice(rest);
    blake2b_compress(&mut state, &block, counter, true);
    state.iter().flat_map(|word| word.to_le_bytes()).take(length).collect()
}

/// BLAKE3's domain flags
const CHUNK_START: u32 = 1;
const CHUNK_END: u32 = 2;
const PARENT: u32 = 4;
const ROOT: u32 = 8;
/// How BLAKE3 reorders the message words between rounds
const BLAKE3_PERMUTATION: [usize; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];
const BLAKE3_BLOCK: usize = 64;
const BLAKE3_CHUNK: usize = 1024;

/// BLAKE3's G
fn blake3_mix(v: &mut [u32; 16], [a, b, c, d]: [usize; 4], x: u32, y: u32) {
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
    v[d] = (v[d] ^ v[a]).rotate_right(16);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(12);
    v[a] = v[a].wrapping_add(v[b]).wrapping_add(y);
    v[d] = (v[d] ^ v[a]).rotate_right(8);
    v[c] = v[c].wrapping_add(v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(7);
}

/// BLAKE3's compression of one block (`length` bytes of it used) into the next chaining value: the first eight words of its output,
/// all a 32-byte hash needs
fn blake3_compress(chaining: &[u32; 8], block: &[u32; 16], counter: u64, length: u32, flags: u32) -> [u32; 8] {
    let mut v = [0u32; 16];
    v[..8].copy_from_slice(chaining);
    v[8..12].copy_from_slice(&SHA256_IV[..4]);
    v[12] = counter as u32;
    v[13] = (counter >> 32) as u32;
    v[14] = length;
    v[15] = flags;
    let mut message = *block;
    for round in 0..7 {
        for (index, &words) in MIXES.iter().enumerate() {
            blake3_mix(&mut v, words, message[2 * index], message[2 * index + 1]);
        }
        if round < 6 {
            message = std::array::from_fn(|index| message[BLAKE3_PERMUTATION[index]]);
        }
    }
    std::array::from_fn(|index| v[index] ^ v[index + 8])
}

/// Up to 64 bytes as a block's 16 words, zero-padded
fn blake3_block(bytes: &[u8]) -> [u32; 16] {
    let mut padded = [0u8; BLAKE3_BLOCK];
    padded[..bytes.len()].copy_from_slice(bytes);
    std::array::from_fn(|index| word_le(&padded, index))
}

/// A node's last compression, held until it is known whether the node is the root
struct Blake3Node {
    chaining: [u32; 8],
    block: [u32; 16],
    counter: u64,
    length: u32,
    flags: u32,
}

impl Blake3Node {
    /// One chunk of up to 1,024 bytes (the `index`th), all but its last block compressed
    fn chunk(chunk: &[u8], index: u64) -> Self {
        let blocks = chunk.len().div_ceil(BLAKE3_BLOCK).max(1);
        let mut chaining = SHA256_IV;
        for (number, block) in chunk.chunks(BLAKE3_BLOCK).take(blocks - 1).enumerate() {
            let flags = if number == 0 { CHUNK_START } else { 0 };
            chaining = blake3_compress(&chaining, &blake3_block(block), index, BLAKE3_BLOCK as u32, flags);
        }
        let last = &chunk[BLAKE3_BLOCK * (blocks - 1)..];
        let flags = CHUNK_END | if blocks == 1 { CHUNK_START } else { 0 };
        Self { chaining, block: blake3_block(last), counter: index, length: last.len() as u32, flags }
    }

    fn parent(left: [u32; 8], right: [u32; 8]) -> Self {
        let mut block = [0u32; 16];
        block[..8].copy_from_slice(&left);
        block[8..].copy_from_slice(&right);
        Self { chaining: SHA256_IV, block, counter: 0, length: BLAKE3_BLOCK as u32, flags: PARENT }
    }

    fn chaining_value(&self) -> [u32; 8] {
        blake3_compress(&self.chaining, &self.block, self.counter, self.length, self.flags)
    }

    fn root(&self) -> [u8; 32] {
        word_bytes(&blake3_compress(&self.chaining, &self.block, 0, self.length, self.flags | ROOT), false)
    }
}

/// BLAKE3's hash of `input`, 32 bytes. The chunks form a tree whose left subtrees hold a power of two of them: each finished chunk's
/// chaining value merges with the stack's top while the count of chunks so far is even, as the reference implementation's stack does.
pub fn blake3(input: &[u8]) -> [u8; 32] {
    let chunks = input.len().div_ceil(BLAKE3_CHUNK).max(1);
    let mut stack: Vec<[u32; 8]> = Vec::new();
    for (index, chunk) in input.chunks(BLAKE3_CHUNK).take(chunks - 1).enumerate() {
        let mut chaining = Blake3Node::chunk(chunk, index as u64).chaining_value();
        let mut done = index as u64 + 1;
        while done & 1 == 0 {
            let left = stack.pop().expect("a left subtree");
            chaining = Blake3Node::parent(left, chaining).chaining_value();
            done >>= 1;
        }
        stack.push(chaining);
    }
    let mut node = Blake3Node::chunk(&input[BLAKE3_CHUNK * (chunks - 1)..], (chunks - 1) as u64);
    while let Some(left) = stack.pop() {
        node = Blake3Node::parent(left, node.chaining_value());
    }
    node.root()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// BLAKE3's test vectors' input: byte i is i % 251
    fn pattern(length: usize) -> Vec<u8> {
        (0..length).map(|index| (index % 251) as u8).collect()
    }

    const MILLION_A: usize = 1_000_000;

    /// RFC 1321's test suite (its appendix A.5)
    #[test]
    fn md5_gives_rfc_1321s_digests() {
        for (input, digest) in [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            ("abcdefghijklmnopqrstuvwxyz", "c3fcd3d76192e4007dfb496cca67e13b"),
            ("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789", "d174ab98d277d9f5a5611c2c9f419d9f"),
            ("12345678901234567890123456789012345678901234567890123456789012345678901234567890", "57edf4a22be3c955ac49da2e2107b67a"),
        ] {
            assert_eq!(hex(&md5(input.as_bytes())), digest, "{input:?}");
        }
    }

    /// FIPS 180's examples (one block, two blocks, a million "a"), the empty input's well-known digests, and RFC 3174's
    /// "01234567" x 80 for SHA-1
    #[test]
    fn sha1_and_sha256_give_the_published_digests() {
        let two_blocks = "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        let million = "a".repeat(MILLION_A);
        let eighty = "01234567".repeat(80);
        for (input, digest) in [
            ("", "da39a3ee5e6b4b0d3255bfef95601890afd80709"),
            ("abc", "a9993e364706816aba3e25717850c26c9cd0d89d"),
            (two_blocks, "84983e441c3bd26ebaae4aa1f95129e5e54670f1"),
            (&million, "34aa973cd4c4daa4f61eeb2bdbad27316534016f"),
            (&eighty, "dea356a2cddd90c7a7ecedc5ebb563934f460452"),
        ] {
            assert_eq!(hex(&sha1(input.as_bytes())), digest, "{} bytes", input.len());
        }
        for (input, digest) in [
            ("", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            ("abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
            (two_blocks, "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"),
            (&million, "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"),
        ] {
            assert_eq!(hex(&sha256(input.as_bytes())), digest, "{} bytes", input.len());
        }
    }

    /// RFC 7693's example (BLAKE2b-512 of "abc", its appendix A), the empty input's BLAKE2b-512, and BLAKE2b-256's digests of the
    /// empty input and "abc" (the length a client's HashAlgorithm.Blake2b answers)
    #[test]
    fn blake2b_gives_the_published_digests_at_each_length() {
        assert_eq!(
            hex(&blake2b(b"abc", 64)),
            "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923"
        );
        assert_eq!(
            hex(&blake2b(b"", 64)),
            "786a02f742015903c6c6fd852552d272912f4740e15847618a86e217f71f5419d25e1031afee585313896444934eb04b903a685b1448b755d56f701afe9be2ce"
        );
        assert_eq!(hex(&blake2b(b"", 32)), "0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8");
        assert_eq!(hex(&blake2b(b"abc", 32)), "bddd813c634239723171ef3fee98579b94964e3bb1cb3e427262c8c068d52319");
    }

    /// BLAKE3's published test vectors (BLAKE3-team/BLAKE3 test_vectors/test_vectors.json, the hash mode's first 32 bytes): one
    /// block, several, one chunk, several, and trees of up to 100 chunks
    #[test]
    fn blake3_gives_its_test_vectors() {
        for (length, digest) in [
            (0, "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"),
            (1, "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213"),
            (2, "7b7015bb92cf0b318037702a6cdd81dee41224f734684c2c122cd6359cb1ee63"),
            (3, "e1be4d7a8ab5560aa4199eea339849ba8e293d55ca0a81006726d184519e647f"),
            (4, "f30f5ab28fe047904037f77b6da4fea1e27241c5d132638d8bedce9d40494f32"),
            (5, "b40b44dfd97e7a84a996a91af8b85188c66c126940ba7aad2e7ae6b385402aa2"),
            (6, "06c4e8ffb6872fad96f9aaca5eee1553eb62aed0ad7198cef42e87f6a616c844"),
            (7, "3f8770f387faad08faa9d8414e9f449ac68e6ff0417f673f602a646a891419fe"),
            (8, "2351207d04fc16ade43ccab08600939c7c1fa70a5c0aaca76063d04c3228eaeb"),
            (63, "e9bc37a594daad83be9470df7f7b3798297c3d834ce80ba85d6e207627b7db7b"),
            (64, "4eed7141ea4a5cd4b788606bd23f46e212af9cacebacdc7d1f4c6dc7f2511b98"),
            (65, "de1e5fa0be70df6d2be8fffd0e99ceaa8eb6e8c93a63f2d8d1c30ecb6b263dee"),
            (127, "d81293fda863f008c09e92fc382a81f5a0b4a1251cba1634016a0f86a6bd640d"),
            (128, "f17e570564b26578c33bb7f44643f539624b05df1a76c81f30acd548c44b45ef"),
            (129, "683aaae9f3c5ba37eaaf072aed0f9e30bac0865137bae68b1fde4ca2aebdcb12"),
            (1023, "10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11"),
            (1024, "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7"),
            (1025, "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444"),
            (2048, "e776b6028c7cd22a4d0ba182a8bf62205d2ef576467e838ed6f2529b85fba24a"),
            (2049, "5f4d72f40d7a5f82b15ca2b2e44b1de3c2ef86c426c95c1af0b6879522563030"),
            (3072, "b98cb0ff3623be03326b373de6b9095218513e64f1ee2edd2525c7ad1e5cffd2"),
            (3073, "7124b49501012f81cc7f11ca069ec9226cecb8a2c850cfe644e327d22d3e1cd3"),
            (4096, "015094013f57a5277b59d8475c0501042c0b642e531b0a1c8f58d2163229e969"),
            (4097, "9b4052b38f1c5fc8b1f9ff7ac7b27cd242487b3d890d15c96a1c25b8aa0fb995"),
            (5120, "9cadc15fed8b5d854562b26a9536d9707cadeda9b143978f319ab34230535833"),
            (5121, "628bd2cb2004694adaab7bbd778a25df25c47b9d4155a55f8fbd79f2fe154cff"),
            (6144, "3e2e5b74e048f3add6d21faab3f83aa44d3b2278afb83b80b3c35164ebeca205"),
            (6145, "f1323a8631446cc50536a9f705ee5cb619424d46887f3c376c695b70e0f0507f"),
            (7168, "61da957ec2499a95d6b8023e2b0e604ec7f6b50e80a9678b89d2628e99ada77a"),
            (7169, "a003fc7a51754a9b3c7fae0367ab3d782dccf28855a03d435f8cfe74605e7817"),
            (8192, "aae792484c8efe4f19e2ca7d371d8c467ffb10748d8a5a1ae579948f718a2a63"),
            (8193, "bab6c09cb8ce8cf459261398d2e7aef35700bf488116ceb94a36d0f5f1b7bc3b"),
            (16384, "f875d6646de28985646f34ee13be9a576fd515f76b5b0a26bb324735041ddde4"),
            (31744, "62b6960e1a44bcc1eb1a611a8d6235b6b4b78f32e7abc4fb4c6cdcce94895c47"),
            (102400, "bc3e3d41a1146b069abffad3c0d44860cf664390afce4d9661f7902e7943e085"),
        ] {
            assert_eq!(hex(&blake3(&pattern(length))), digest, "{length} bytes");
        }
    }
}
