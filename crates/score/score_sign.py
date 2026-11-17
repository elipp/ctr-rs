"""Independent signer for `score`'s pinned test (`signs_as_pinned`): RFC 8032's reference
Ed25519, stdlib only. Run: python3 crates/score/score_sign.py"""

import hashlib

p = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
d = -121665 * pow(121666, -1, p) % p
I = pow(2, (p - 1) // 4, p)


def xrecover(y):
    xx = (y * y - 1) * pow(d * y * y + 1, -1, p)
    x = pow(xx, (p + 3) // 8, p)
    if (x * x - xx) % p:
        x = x * I % p
    return p - x if x % 2 else x


By = 4 * pow(5, -1, p) % p
B = (xrecover(By), By, 1, xrecover(By) * By % p)


def add(P, Q):
    x1, y1, z1, t1 = P
    x2, y2, z2, t2 = Q
    a = (y1 - x1) * (y2 - x2) % p
    b = (y1 + x1) * (y2 + x2) % p
    c = t1 * 2 * d * t2 % p
    dd = z1 * 2 * z2 % p
    e, f, g, h = b - a, dd - c, dd + c, b + a
    return (e * f % p, g * h % p, f * g % p, e * h % p)


def mul(s, P):
    Q = (0, 1, 1, 0)
    while s:
        if s & 1:
            Q = add(Q, P)
        P = add(P, P)
        s >>= 1
    return Q


def enc(P):
    x, y, z, _ = P
    zi = pow(z, -1, p)
    x, y = x * zi % p, y * zi % p
    return (y | (x & 1) << 255).to_bytes(32, "little")


def h(m):
    return int.from_bytes(hashlib.sha512(m).digest(), "little")


def sign(secret, msg):
    hs = hashlib.sha512(secret).digest()
    a = int.from_bytes(hs[:32], "little")
    a &= (1 << 254) - 8
    a |= 1 << 254
    A = enc(mul(a, B))
    r = h(hs[32:] + msg) % L
    R = enc(mul(r, B))
    s = (r + h(R + A + msg) * a) % L
    return A, R + s.to_bytes(32, "little")


secret = bytes.fromhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
A, sig = sign(secret, b"")
assert A.hex() == "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
assert sig.hex().startswith("e5564300c360ac72")

body, ts = b"CTRR run bytes", 1_800_000_000
message = f"ctr-score/1\nPOST\n/runs\n{ts}\n{hashlib.sha256(body).hexdigest()}"
_, sig = sign(secret, message.encode())
print("message:", repr(message))
print(f"header: CTR-Ed25519 key={A.hex()}, ts={ts}, sig={sig.hex()}")
