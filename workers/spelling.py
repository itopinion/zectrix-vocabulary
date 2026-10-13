"""Answer-blind spelling transcription and optional pitch-preserving slow replay."""
import re
import numpy as np

SPELLING_CONTEXT = (
    'The recording may contain English alphabet letter names spoken in sequence during a spelling exercise. '
    'Transcribe each spoken letter separately, separated by spaces. Preserve repetitions, omissions and the actual order. '
    'Do not correct spelling, complete a word, or convert a whole spoken word into letters. '
    'If a whole word is spoken, transcribe it as a whole word.'
)

READING_CONTEXT = (
    'An English vocabulary word or phrase is being read aloud. '
    'Transcribe the spoken word verbatim, without correcting or adding words. '
    'This is pronunciation practice, not spelling letter names.'
)

def letters(text, minimum=2):
    tokens = re.findall('[A-Za-z]+', text)
    # Match the Rust judge; an ordinary spoken word is not proof of spelling.
    return ''.join(tokens).upper() if len(tokens) >= minimum and all(len(t) == 1 for t in tokens) else None


def slow_replay(audio, rate=0.78):
    """WSOLA: overlap waveform-aligned frames without shifting letter-name pitch.

    40 ms frames, 20 ms output steps, an 8 ms alignment search. No subprocess,
    downloads or additional speech model. The recorded original is unchanged.
    """
    audio = np.asarray(audio, dtype=np.float32)
    hop, frame, search = 320, 640, 128
    length = int(len(audio) / rate)
    source = np.pad(audio, (0, frame + search))
    output = np.zeros(length + frame, dtype=np.float32)
    output[:frame] = source[:frame]
    fade = np.linspace(0, 1, hop, dtype=np.float32)
    for pos in range(hop, length, hop):
        target = int(pos * rate)
        low, high = max(0, target-search), min(len(source)-frame, target+search)
        prior = output[pos:pos+hop]
        candidates = np.lib.stride_tricks.sliding_window_view(source[low:high+hop], hop)
        if np.dot(prior, prior) > 1e-7 and len(candidates):
            norms = np.sqrt(np.einsum('ij,ij->i', candidates, candidates) + 1e-10)
            correlation = (candidates @ prior) / norms
            chosen = low + int(np.argmax(correlation))
        else:
            chosen = target
        chunk = source[chosen:chosen+frame]
        output[pos:pos+hop] = prior * (1-fade) + chunk[:hop] * fade
        output[pos+hop:pos+frame] = chunk[hop:]
    return output[:length]


def pause_boundaries(audio):
    # Fast letter names may have quieter valleys rather than complete silence.
    # Keep original cuts, then add clearly distinct cuts from a higher threshold;
    # nearby variants must not count as separate corroborating partitions.
    original = _pause_boundaries(audio, .13)
    extra = _pause_boundaries(audio, .30)
    for cut in extra[1:-1]:
        if all(abs(cut - other) >= 2400 for other in original):
            original.append(cut)
    return sorted(original)


def _pause_boundaries(audio, ratio):
    """Find short natural pauses, without assuming a word or a letter count."""
    audio = np.asarray(audio, dtype=np.float32)
    rms = np.sqrt(np.mean(np.pad(audio, (0, (-len(audio)) % 160)).reshape(-1, 160) ** 2, axis=1))
    threshold = max(.003, float(np.percentile(rms, 90)) * ratio)
    quiet = rms < threshold
    boundaries = [0]
    start = None
    for i, silent in enumerate(np.r_[quiet, False]):
        if silent and start is None:
            start = i
        elif not silent and start is not None:
            if i - start >= 3 and start > 15 and i < len(rms) - 10:
                middle = (start + i) * 80
                if middle - boundaries[-1] >= 2400:
                    boundaries.append(middle)
            start = None
    boundaries.append(len(audio))
    return boundaries


def candidate_partitions(bounds):
    """Short disjoint chunks at acoustic pauses; no assumption about letter count."""
    paths = [[] for _ in bounds]
    paths[0] = [(0.0, (0,))]
    for end in range(1, len(bounds)):
        candidates = []
        for start in range(end):
            duration = (bounds[end] - bounds[start]) / 16000
            if not .45 <= duration <= 1.8:
                continue
            for cost, path in paths[start]:
                if len(path) <= 8:
                    candidates.append((cost + (duration - 1.1)**2 + .12, path + (end,)))
        paths[end] = sorted(candidates)[:24]
    return [path for _, path in paths[-1] if len(path) >= 3]


def segmented_replay(model, audio):
    """Accept agreement of two different complete, disjoint transcriptions.

    Pauses are candidate cuts, not a proxy for the number of letters. Every
    route covers the original recording once, so repeated letters survive.
    Unresolved chunks and disagreement never get repaired from the answer.
    """
    bounds = pause_boundaries(audio)
    passes = []
    if not 3 <= len(bounds) <= 42:
        return None, passes
    routes = candidate_partitions(bounds)
    cache, votes = {}, {}
    first_route = None
    for route_index in range(len(routes)):
        # Once the call budget is spent, inspect all remaining routes that can
        # be assembled from cached chunks; skipped routes must not hide them.
        if route_index >= 12 and len(cache) < 18:
            break
        if first_route is None:
            path = routes.pop(0)
            first_route = path
        else:
            # Prefer a second route with different cut positions.
            chosen = min(range(len(routes)), key=lambda i: (len(set(routes[i][1:-1]) & set(first_route[1:-1])), i))
            path = routes.pop(chosen)
        pieces = []
        for start, end in zip(path, path[1:]):
            key = (start, end)
            if key not in cache:
                if len(cache) >= 18:
                    break
                chunk = np.pad(audio[bounds[start]:bounds[end]], (3200, 4800))
                text = model.generate(chunk, language='English', max_tokens=64, system_prompt=SPELLING_CONTEXT).text
                # A complete spelling needs multiple letters, but a short
                # fragment may contain just one (e.g. CAM + P).
                cache[key] = letters(text, minimum=1)
                passes.append({'method':'disjoint_segment', 'start':bounds[start]/16000,
                               'end':bounds[end]/16000, 'text':text})
            sequence = cache[key]
            if not sequence:
                break
            pieces.append(sequence)
        else:
            sequence = ''.join(pieces)
            passes.append({'method':'complete_partition', 'cuts':[bounds[i]/16000 for i in path],
                           'text':' '.join(sequence)})
            votes[sequence] = votes.get(sequence, 0) + 1
            if len(sequence) >= 2 and votes[sequence] >= 2:
                return ' '.join(sequence), passes
    return None, passes


def transcribe(model, path, mode):
    if mode != 'spell':
        result = model.generate(path, language='English', max_tokens=192, system_prompt=READING_CONTEXT)
        return {'text':result.text, 'recognition':{'method':'word_reading',
                'passes':[{'method':'vocabulary_context','text':result.text}]}}
    first = model.generate(path, language='English', max_tokens=192, system_prompt=SPELLING_CONTEXT).text
    passes = [{'method':'spelling', 'text':first}]
    if letters(first):
        return {'text':first, 'recognition':{'method':'spelling','passes':passes}}
    import soundfile as sf
    audio, sr = sf.read(path, dtype='float32')
    if sr != 16000 or audio.ndim != 1:
        raise ValueError('Spelling expects mono 16 kHz audio')
    # Retry only an unresolved spelling sequence; never use the expected answer
    # to choose between interpretations. Silence must stay unresolved.
    if not len(audio) or float(np.max(np.abs(audio))) < 0.003:
        return {'text':first, 'recognition':{'method':'unresolved','passes':passes}}
    slowed = slow_replay(audio)
    slowed = np.pad(slowed, (2400, 4000))
    try:
        second = model.generate(slowed, language='English', max_tokens=192, system_prompt=SPELLING_CONTEXT).text
        passes.append({'method':'slow_replay','text':second})
    except Exception as error:
        return {'text':first,'recognition':{'method':'unresolved','passes':passes,'retry_error':str(error)}}
    accepted = bool(letters(second))
    if accepted:
        return {'text':second, 'recognition':{'method':'slow_replay','passes':passes}}
    try:
        segmented, details = segmented_replay(model, audio)
        passes.extend(details)
        if segmented:
            return {'text':segmented, 'recognition':{'method':'partition_consensus','passes':passes}}
    except Exception as error:
        return {'text':first,'recognition':{'method':'unresolved','passes':passes,'segment_error':str(error)}}
    return {'text':first, 'recognition':{'method':'unresolved','passes':passes}}
