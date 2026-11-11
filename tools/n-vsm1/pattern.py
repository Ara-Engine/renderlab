"""N-VSM1: the disk lookup's 64 sample directions (src/virtual_shadow_disk_pattern.rs).

The lookup ORs axis-aligned rectangles into a 64-bit mask. For that to cost a few shifts, the
samples are a rook pattern in the disk: sample s has the s-th smallest x, and its y rank is a
permutation. Both coordinates sit exactly at the 64 quantiles of the disk's marginal
distribution, so "samples with x below g" is a rank computed from the marginal's cumulative
distribution, and an axis-aligned occluder edge sweeping the disk passes one sample at a time
(64 steps, where an 8x8 grid gives 8).

    py -3 -B tools/n-vsm1/pattern.py            # prints the Rust table and the pattern's measures

The permutation is found by a seeded search: start from a sunflower spiral's ranks, then swap
y ranks while the smallest distance between samples grows and every sample stays in the disk.
"""
import math
import random


def cdf(g):
    """Share of a unit disk's area with x below g."""
    g = max(-1.0, min(1.0, g))
    return 0.5 + (g * math.sqrt(1.0 - g * g) + math.asin(g)) / math.pi


def quantile(u):
    lo, hi = -1.0, 1.0
    for _ in range(60):
        mid = 0.5 * (lo + hi)
        lo, hi = (mid, hi) if cdf(mid) < u else (lo, mid)
    return 0.5 * (lo + hi)


N = 64
Q = [quantile((k + 0.5) / N) for k in range(N)]


def points(order):
    return [(Q[s], Q[order[s]]) for s in range(N)]


def spread(order):
    """(samples outside the disk, smallest squared distance): fewer outside, then wider apart."""
    p = points(order)
    outside = sum(x * x + y * y > 1.0 for x, y in p)
    nearest = min((ax - bx) ** 2 + (ay - by) ** 2 for i, (ax, ay) in enumerate(p) for bx, by in p[:i])
    return (-outside, nearest)


def search(seed=1, rounds=200_000):
    golden = math.pi * (3.0 - math.sqrt(5.0))
    spiral = [(math.sqrt((i + 0.5) / N) * math.cos(i * golden), math.sqrt((i + 0.5) / N) * math.sin(i * golden)) for i in range(N)]
    by_x = sorted(range(N), key=lambda i: spiral[i][0])
    y_rank = {i: r for r, i in enumerate(sorted(range(N), key=lambda i: spiral[i][1]))}
    order = [y_rank[i] for i in by_x]
    rng = random.Random(seed)
    best = spread(order)
    for _ in range(rounds):
        a, b = rng.randrange(N), rng.randrange(N)
        if a == b:
            continue
        order[a], order[b] = order[b], order[a]
        score = spread(order)
        if score >= best:
            best = score
        else:
            order[a], order[b] = order[b], order[a]
    return order, best


if __name__ == "__main__":
    order, (outside, nearest) = search()
    p = points(order)
    print(f"// outside the disk: {-outside}; nearest pair {math.sqrt(nearest):.4f} (a square lattice of 64 in the disk: {math.sqrt(math.pi / N):.4f})")
    print(f"// largest radius {max(math.hypot(x, y) for x, y in p):.4f}")
    print("pub(super) const Y_RANK: [u8; 64] = [")
    for row in range(0, N, 16):
        print("    " + ", ".join(str(v) for v in order[row:row + 16]) + ",")
    print("];")
    print("pub(super) const QUANTILE: [f32; 64] = [")
    for row in range(0, N, 8):
        print("    " + ", ".join(f"{v:.7f}" for v in Q[row:row + 8]) + ",")
    print("];")
