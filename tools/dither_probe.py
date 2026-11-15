#!/usr/bin/env python3
"""What a decoder draws where a stream has no bits, read off what it writes.

A mantissa without bits takes a random value when its channel asks for dither
(ATSC A/52 clause 7.3.4), and the standard leaves the sequence to the decoder.
A decoder that writes the same samples every time it decodes a stream draws a
sequence that can be read: write a stream in which no mantissa has a bit, so
that what comes out is the dither and nothing else, and take the output back
through the transform to the coefficients it was made of.

    cargo run --release -p oadec-synth --example dither_probe -- probe.ac3
    <the decoder> probe.ac3 -> probe.f32      (32-bit float, the channels interleaved)
    python tools/dither_probe.py read probe.f32 --out probe.json

`read` says, of the decode of that stream (every exponent 8):

- how fine the values are: the power of two that makes whole numbers of them;
- how they are spread;
- after how many values the run comes round again, and what two readings of
  one place differ by where they differ;
- the rule that takes the rank of one value to the rank of the next, which is
  the generator behind them when every state is visited once;
- which generator of 16 bits, `x -> x * M + C` with two draws a value, gives
  every block of every channel as a run, searched over every `C` and the `M`
  the rule allows;
- the state each block of each channel starts from, how far a block has moved
  one frame on, and where the states of the first frame stand on the run of
  `g -> g * M + 1` from 0.

The decoder of Dolby's reference player leaves out the first block of a
stream, so block `b` of its decode is block `b + 1` of the stream; that is
what `--first-block` says (1 when absent, 0 for a decode that starts at the
start).

    python tools/dither_probe.py against reference.f32 with.wav without.wav --channels 6

`against` is for any stream: `with.wav` and `without.wav` are this decoder's
decode with its dither and without (`oadec decode`, `oadec decode
--no-dither`), and `reference.f32` another decoder's. For every channel: the
level of what the reference adds to the decode without dither, of what this
decoder adds, and of what is left of the reference's once this decoder's is
taken out. Where the two draw the same values the last is nothing but the
rounding of the reference's 32-bit floats. `--lag` is how many samples into
this decoder's decode the reference starts (256 when absent).

    python tools/dither_probe.py --self-check

makes the decode of a probe stream from the generator this file holds and
reads it back.

The decoder under study is run as it stands and only what it writes is read.
Exit code: 0, 1 when a decode cannot be read as asked, 2 on a usage error.
"""
import argparse
import json
import sys
from collections import Counter, defaultdict, deque
from math import gcd
from pathlib import Path

import numpy as np

N = 512  # samples a block is transformed from
HALF = N // 2
BLOCKS = 6  # blocks of a frame
SLOTS = 5  # full-bandwidth channels a frame may have

# The generator, written here a second time (crates/oadec-eac3/src/dither.rs).
MULTIPLIER = 47989
DRAW = 3
SEED = 1
SCALE = 23170  # 1/sqrt(2) in units of 2^-15


def window():
    """The 256 rising values of the window of clause 7.9.5: Kaiser-Bessel
    derived, alpha 5."""
    j = np.arange(HALF + 1)
    kaiser = np.i0(np.pi * 5.0 * np.sqrt(1 - (2 * j / HALF - 1) ** 2)) / np.i0(np.pi * 5.0)
    return np.sqrt(np.cumsum(kaiser)[:HALF] / kaiser.sum())


WINDOW = np.concatenate([window(), window()[::-1]])
# X[k] = -2/N sum x[n] cos(2 pi (2n + 1)(2k + 1) / 4N + pi (2k + 1) / 4), clause 8.2.3.2
_K, _T = np.arange(HALF), np.arange(N)
BASIS = np.cos(2 * np.pi * np.outer(2 * _K + 1, 2 * _T + 1) / (4 * N) + np.pi * (2 * _K + 1)[:, None] / 4)


def coefficients(pcm):
    """The coefficients of every block of one channel: row b is the block
    whose 512 samples start at 256 b. Windowed overlap-add with this window is
    an orthogonal transform, so what comes back is what the decoder put in."""
    count = (len(pcm) - N) // HALF + 1
    out = np.empty((max(count, 0), HALF))
    for b in range(count):
        out[b] = (-2.0 / N) * (BASIS @ (pcm[b * HALF:b * HALF + N] * WINDOW))
    return out


def samples(blocks):
    """The inverse: the samples that blocks of coefficients come to."""
    out = np.zeros((len(blocks) + 1) * HALF)
    for b, row in enumerate(blocks):
        out[b * HALF:b * HALF + N] += -2.0 * WINDOW * (row @ BASIS)
    return out


def read_decode(path, channels):
    raw = Path(path).read_bytes()
    if raw[:4] in (b'RIFF', b'RF64'):
        at = raw.find(b'data')
        if at < 0:
            sys.exit(f'{path}: a WAVE file without a data chunk')
        size = int.from_bytes(raw[at + 4:at + 8], 'little')
        raw = raw[at + 8:] if size in (0, 0xFFFFFFFF) else raw[at + 8:at + 8 + size]
    usable = len(raw) // (4 * channels) * 4 * channels
    return np.frombuffer(raw[:usable], dtype='<f4').reshape(-1, channels).astype(np.float64)


def signed(x):
    return ((x + 32768) & 0xFFFF) - 32768


def value(a, b):
    """The value two draws come to, in units of 2^-15."""
    return (((signed(a) >> 1) + (signed(b) >> 1)) * SCALE) >> 15


def run(state, count, multiplier=MULTIPLIER, draw=DRAW):
    """`count` values from `state`, and the state after them."""
    out = []
    for _ in range(count):
        a = (state * multiplier + draw) & 0xFFFF
        state = (a * multiplier + draw) & 0xFFFF
        out.append(value(a, state))
    return out, state


def seed_states():
    """The states a decode starts from: own and coupled, by block and channel."""
    own = [[0] * SLOTS for _ in range(BLOCKS)]
    coupled = [[0] * SLOTS for _ in range(BLOCKS)]
    g = 1
    for blk in range(BLOCKS):
        for ch in range(SLOTS):
            g = (g * MULTIPLIER + SEED) & 0xFFFF
            own[blk][ch] = g
            g = (g * MULTIPLIER + SEED) & 0xFFFF
            coupled[blk][ch] = g
    return own, coupled


def values_of(pcm, exponent, counts):
    """The values of every block and channel as the decoder drew them, over
    the exponent: `x[block][channel]`, an array of the channel's coefficients."""
    per = [coefficients(pcm[:, ch]) for ch in range(pcm.shape[1])]
    return [[per[ch][b, :counts[ch]] * 2.0 ** exponent for ch in range(pcm.shape[1])] for b in range(len(per[0]))]


def how_fine(x):
    """The least power of two that makes whole numbers of the values."""
    flat = np.concatenate([row for block in x for row in block])
    for bits in range(8, 25):
        scaled = flat * 2.0 ** bits
        off = float(np.abs(scaled - np.round(scaled)).max())
        if off < 0.05:
            return bits, off
    return None, None


def the_cycle(k, length=6):
    """Lays every block of every channel out on the one run they are read
    from: where two stretches say the same `length` values they are the same
    place of it. Returns after how many values the run closes (0 if it does
    not), where each stretch starts on it, and the readings of every place."""
    seen = defaultdict(list)
    stretches = [(b, ch) for b in range(len(k)) for ch in range(len(k[b])) if len(k[b][ch]) >= length]
    for s, (b, ch) in enumerate(stretches):
        row = k[b][ch]
        for i in range(len(row) - length + 1):
            seen[row[i:i + length].tobytes()].append((s, i))
    edges = defaultdict(dict)
    for where in seen.values():
        s0, i0 = where[0]
        for s1, i1 in where[1:]:
            if s1 != s0:
                edges[s0].setdefault(s1, set()).add(i0 - i1)
                edges[s1].setdefault(s0, set()).add(i1 - i0)
    start, conflicts = {0: 0}, []
    queue = deque([0])
    while queue:
        a = queue.popleft()
        for other, shifts in edges[a].items():
            for d in shifts:
                if other not in start:
                    start[other] = start[a] + d
                    queue.append(other)
                elif start[other] != start[a] + d:
                    conflicts.append(abs(start[other] - start[a] - d))
    period = 0
    for c in conflicts:
        period = gcd(period, c)
    readings = defaultdict(Counter)
    if period:
        for s, at in start.items():
            b, ch = stretches[s]
            for i, v in enumerate(k[b][ch]):
                readings[(at + i) % period][int(v)] += 1
    return period, len(start), len(stretches), readings


def the_recurrence(readings, period):
    """The values of a full turn of the run, each taken modulo what two
    readings of a place differ by, fall evenly: one for every place, so they
    are the states of a generator that visits every state once. Their ranks
    say how one state follows from another: rank' = a * rank + c."""
    apart = Counter()
    for counts in readings.values():
        vals = sorted(counts)
        for lo, hi in zip(vals[:-1], vals[1:]):
            apart[hi - lo] += 1
    if not apart or len(readings) < period:
        return None
    modulus = apart.most_common(1)[0][0]
    table = np.array([readings[p].most_common(1)[0][0] for p in range(period)])
    w = table % modulus
    ranks = np.unique(w)
    if len(ranks) != period:
        return {'two_readings_differ_by': int(modulus), 'different_values_modulo_it': int(len(ranks))}
    rank = np.searchsorted(ranks, w)
    where = {int(r): i for i, r in enumerate(rank)}
    a = Counter(int((rank[(where[r + 1] + 1) % period] - rank[(where[r] + 1) % period]) % period)
                for r in range(64)).most_common(1)[0][0]
    constants = Counter(((np.roll(rank, -1) - a * rank) % period).tolist())
    return {
        'two_readings_differ_by': int(modulus),
        'the_value_modulo_it_is_floor_of_rank_times_it_over_the_period':
            bool((w == (rank * modulus) // period).all()),
        'multiplier': a,
        'increment': constants.most_common(1)[0][0],
        'places_that_follow_the_rule': int(constants.most_common(1)[0][1]),
        'places': int(period),
    }


def generators(multiplier_squared, increment, period):
    """Every `x -> x * M + C` over 16 bits whose pairs of draws, summed and
    cut by two bits, follow `q -> q * multiplier_squared + increment` modulo
    `period`: the candidates the search has to tell apart."""
    out = []
    c = np.arange(65536, dtype=np.int64)
    for m in range(1, 65536, 2):
        if (m * m - multiplier_squared) % period:
            continue
        for x0 in range(4):
            x = np.full(65536, x0, dtype=np.int64)
            q = []
            for _ in range(4):
                a = (x * m + c) & 0xFFFF
                x = (a * m + c) & 0xFFFF
                q.append(((a + x) & 0xFFFF) >> 2)
            fits = np.ones(65536, dtype=bool)
            for i in range(3):
                fits &= q[i + 1] == (q[i] * multiplier_squared + increment) % period
            out.extend((m, int(v)) for v in np.nonzero(fits)[0])
    return sorted(set(out))


def stretches_that_fit(k, multiplier, draw):
    """How many blocks of channels are a run of this generator, and the state
    each starts from."""
    x = np.arange(65536, dtype=np.int64)
    a = (x * multiplier + draw) & 0xFFFF
    b = (a * multiplier + draw) & 0xFFFF
    first = value(a, b)
    by_value = defaultdict(list)
    for state in range(65536):
        by_value[int(first[state])].append(state)
    states, fit = {}, 0
    for blk, block in enumerate(k):
        for ch, row in enumerate(block):
            if not len(row) or not row.any():
                continue
            for state in by_value.get(int(row[0]), []):
                got, _ = run(state, len(row), multiplier, draw)
                if got == row.tolist():
                    states[(blk, ch)] = state
                    fit += 1
                    break
    return fit, states


def study(pcm, exponent, counts, first_block, search=True):
    x = values_of(pcm, exponent, counts)
    bits, off = how_fine(x)
    report = {'blocks_read': len(x), 'channels': pcm.shape[1],
              'whole_numbers_at_two_to_the_minus': bits, 'furthest_from_one': off}
    if bits is None:
        return report
    k = [[np.round(row * 2.0 ** bits).astype(np.int64) for row in block] for block in x]
    flat = np.concatenate([row for block in k for row in block if row.any()]).astype(np.float64) / 2.0 ** bits
    report['spread'] = {
        'least': float(flat.min()), 'most': float(flat.max()),
        'power': float((flat ** 2).mean()),
        'share_within_0.2071': float((np.abs(flat) < 0.2071).mean()),
        'of_an_even_spread_within_0.5': {'power': 1 / 12, 'share_within_0.2071': 0.4142},
        'of_a_sum_of_two_within_0.7071': {'power': 1 / 12, 'share_within_0.2071': 0.5},
    }
    period, laid, of, readings = the_cycle(k)
    report['run'] = {
        'closes_after': period, 'stretches_laid_out': laid, 'of': of,
        'places_read': len(readings),
        'places_with_two_readings': sum(1 for c in readings.values() if len(c) > 1),
    }
    rule = the_recurrence(readings, period) if period else None
    report['rule_of_the_ranks'] = rule
    if search and rule and 'multiplier' in rule:
        found = []
        for m, c in generators(rule['multiplier'], rule['increment'], period):
            fit, _ = stretches_that_fit(k, m, c)
            found.append({'multiplier': m, 'increment': c, 'stretches_that_are_a_run_of_it': fit})
        found.sort(key=lambda g: -g['stretches_that_are_a_run_of_it'])
        report['generators_tried'] = len(found)
        report['generators'] = found[:4]
    fit, states = stretches_that_fit(k, MULTIPLIER, DRAW)
    stretches = sum(1 for block in k for row in block if len(row) and row.any())
    report['the_generator_of_this_decoder'] = {
        'multiplier': MULTIPLIER, 'increment': DRAW, 'stretches_that_are_a_run_of_it': fit, 'of': stretches}
    # where the states of the first frame stand on the seed chain from 0, and
    # how far a block has moved one frame on
    link, g = {}, 0
    for i in range(65536):
        link.setdefault(g, i)
        g = (g * MULTIPLIER + SEED) & 0xFFFF
    position, x_ = {}, 0
    for i in range(65536):
        position[x_] = i
        x_ = (x_ * MULTIPLIER + DRAW) & 0xFFFF
    seeds, moves = [], Counter()
    for (blk, ch), state in sorted(states.items()):
        stream_block = blk + first_block
        frame, of_frame = divmod(stream_block, BLOCKS)
        if frame == 0:
            seeds.append({'block': of_frame, 'channel': ch, 'state': state, 'steps_of_the_seed_generator_from_0': link.get(state)})
        later = states.get((blk + BLOCKS, ch))
        if later is not None:
            moves[(ch, ((position[later] - position[state]) % 65536) // 2)] += 1
    report['states_of_the_first_frame'] = seeds
    report['values_a_block_moves_in_a_frame'] = [
        {'channel': ch, 'values': v, 'coefficients_of_the_channel': counts[ch], 'blocks': n}
        for (ch, v), n in sorted(moves.items())]
    return report


def against(reference, with_dither, without, lag):
    n = min(len(reference), len(with_dither) - lag, len(without) - lag)
    if n <= 48000:
        sys.exit('less than a second to compare')
    ref = reference[:n]
    on, off = with_dither[lag:lag + n], without[lag:lag + n]
    rows = []
    for ch in range(ref.shape[1]):
        def level(x):
            power = float((x ** 2).mean())
            return None if power <= 0 else round(10 * np.log10(power), 2)
        theirs, ours, left = level(ref[:, ch] - off[:, ch]), level(on[:, ch] - off[:, ch]), level(ref[:, ch] - on[:, ch])
        rows.append({
            'channel': ch,
            'the_reference_adds_dbfs': theirs,
            'this_decoder_adds_dbfs': ours,
            'left_of_the_reference_dbfs': left,
            'the_reference_over_what_is_left_db':
                None if theirs is None else (200.0 if left is None else round(theirs - left, 2)),
        })
    return {'samples_compared': n, 'lag': lag, 'channels': rows}


def self_check():
    """A decode of the probe stream made from the generator, read back."""
    frames, counts, exponent = 40, [253, 133], 8
    own, _ = seed_states()
    blocks = [[np.zeros(HALF) for _ in range(frames * BLOCKS)] for _ in range(2)]
    for f in range(frames):
        for blk in range(BLOCKS):
            for ch in range(2):
                got, own[blk][ch] = run(own[blk][ch], counts[ch])
                blocks[ch][f * BLOCKS + blk][:counts[ch]] = np.array(got) / 32768 / 2.0 ** exponent
    # what a decoder that leaves out the first block writes, as 32-bit floats
    pcm = np.stack([samples(blocks[ch])[HALF:] for ch in range(2)], axis=1).astype('<f4').astype(np.float64)
    report = study(pcm, exponent, counts, 1)
    own, _ = seed_states()
    wanted = [{'block': blk, 'channel': ch, 'state': own[blk][ch], 'steps_of_the_seed_generator_from_0': 10 * blk + 2 * ch + 2}
              for blk in range(1, BLOCKS) for ch in range(2)]
    checks = {
        'the values are whole numbers of 2^-15': report['whole_numbers_at_two_to_the_minus'] == 15,
        'they are spread as a sum of two': abs(report['spread']['share_within_0.2071'] - 0.5) < 0.02,
        'the run closes after 16384 values': report['run']['closes_after'] == 16384,
        'two readings of a place differ by 23170':
            (report['rule_of_the_ranks'] or {}).get('two_readings_differ_by') == SCALE,
        'the ranks go q -> 9081 q + 8719':
            ((report['rule_of_the_ranks'] or {}).get('multiplier'), (report['rule_of_the_ranks'] or {}).get('increment')) == (9081, 8719),
        'one generator gives every stretch, and it is 47989 and 3':
            [(g['multiplier'], g['increment']) for g in report.get('generators', [])
             if g['stretches_that_are_a_run_of_it'] == report['the_generator_of_this_decoder']['of']] == [(MULTIPLIER, DRAW)],
        'every stretch is a run of it':
            report['the_generator_of_this_decoder']['stretches_that_are_a_run_of_it'] == report['the_generator_of_this_decoder']['of'],
        'the states of the first frame are the seeds': report['states_of_the_first_frame'] == wanted,
        'a block moves by the coefficients of its channel':
            [(m['channel'], m['values']) for m in report['values_a_block_moves_in_a_frame']] == [(0, 253), (1, 133)],
    }
    # and a decode against itself with another dither: all of the reference's is left
    rng = np.random.default_rng(1)
    silent = rng.standard_normal((len(pcm) + 256, 2)) * 0.01
    theirs = silent[256:] + pcm
    ours = silent.copy()
    ours[256:] += pcm
    same = against(theirs, ours, silent, 256)
    other = against(theirs, silent + rng.standard_normal(silent.shape) * 1e-3, silent, 256)
    checks['the same dither leaves nothing of the reference'] = all(
        c['the_reference_over_what_is_left_db'] > 150 for c in same['channels'])
    checks['another dither leaves all of it'] = all(
        c['the_reference_over_what_is_left_db'] < 1 for c in other['channels'])
    for name, ok in checks.items():
        print(f'{"ok  " if ok else "FAIL"} {name}')
    return 0 if all(checks.values()) else 1


def main():
    parser = argparse.ArgumentParser(description='What a decoder draws where a stream has no bits.')
    parser.add_argument('--self-check', action='store_true')
    commands = parser.add_subparsers(dest='command')
    read = commands.add_parser('read', help='the decode of a probe stream')
    read.add_argument('decode', type=Path, help='32-bit float, headerless or WAVE')
    read.add_argument('--channels', type=int, default=2)
    read.add_argument('--exponent', type=int, default=8)
    read.add_argument('--coefficients', type=int, nargs='+', default=[253],
                      help='the coefficients of each channel (one number for all)')
    read.add_argument('--first-block', type=int, default=1,
                      help='the block of the stream the decode starts with (1: a decoder that leaves out the first)')
    read.add_argument('--no-search', action='store_true', help='leave out the search over the generators')
    read.add_argument('--out', type=Path)
    cmp_ = commands.add_parser('against', help='another decoder against this one, on any stream')
    cmp_.add_argument('reference', type=Path)
    cmp_.add_argument('with_dither', type=Path)
    cmp_.add_argument('without', type=Path)
    cmp_.add_argument('--channels', type=int, required=True)
    cmp_.add_argument('--lag', type=int, default=256)
    cmp_.add_argument('--out', type=Path)
    args = parser.parse_args()
    if args.self_check:
        return self_check()
    if args.command == 'read':
        counts = args.coefficients * args.channels if len(args.coefficients) == 1 else args.coefficients
        if len(counts) != args.channels:
            parser.error('one number of coefficients for each channel, or one for all')
        report = study(read_decode(args.decode, args.channels), args.exponent, counts, args.first_block,
                       not args.no_search)
        if report['whole_numbers_at_two_to_the_minus'] is None:
            print('the values are not whole numbers of any power of two down to 2^-24', file=sys.stderr)
            return 1
    elif args.command == 'against':
        report = against(read_decode(args.reference, args.channels), read_decode(args.with_dither, args.channels),
                         read_decode(args.without, args.channels), args.lag)
    else:
        parser.error('a command, or --self-check')
    text = json.dumps(report, indent=1) + '\n'
    if args.out:
        args.out.write_text(text, encoding='utf-8')
    print(text, end='')
    return 0


if __name__ == '__main__':
    sys.exit(main())
