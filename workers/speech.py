"""Private JSON-lines bridge. All speech stays on this computer."""
import os, sys, json, contextlib, traceback
from pathlib import Path
# The Hub supplies HF_HOME for the shared, user-owned model cache.
os.environ['HF_HUB_OFFLINE']='1'
os.environ['TOKENIZERS_PARALLELISM']='false'
mode=sys.argv[1]
wire=sys.stdout
with contextlib.redirect_stdout(sys.stderr):
    if mode=='asr':
        from spelling import transcribe
        import diagnostics
        from mlx_audio.stt import load
        model=load('mlx-community/Qwen3-ASR-1.7B-8bit')
    else:
        import numpy as np, soundfile as sf
        from scipy.signal import resample_poly
        from kokoro import KPipeline
        model=KPipeline(lang_code='a',repo_id='hexgrad/Kokoro-82M',device='cpu')
print(json.dumps({'ready':True,'model':mode}),file=wire,flush=True)
for line in sys.stdin:
    try:
        req=json.loads(line)
        with contextlib.redirect_stdout(sys.stderr):
            if mode=='asr':
                sample = None
                try:
                    sample = diagnostics.capture(Path(os.environ['ZECTRIX_APP_DATA']), req)
                except Exception:
                    traceback.print_exc(file=sys.stderr)
                # Deliberately do not provide the expected word or textbook as hints.
                out=transcribe(model,req['path'],req.get('mode','read'))
                try:
                    diagnostics.result(sample, out)
                except Exception:
                    traceback.print_exc(file=sys.stderr)
            else:
                chunks=[a.numpy() for _,_,a in model(req['text'],voice='af_heart',speed=0.95)]
                if not chunks: raise ValueError('Kokoro returned empty audio')
                audio=resample_poly(np.concatenate(chunks),2,3)
                dest=Path(req['path']);tmp=dest.with_suffix('.tmp.wav')
                sf.write(tmp,audio,16000,subtype='PCM_16');tmp.replace(dest)
                out={'path':str(dest),'samples':len(audio)}
        print(json.dumps({'ok':True,**out}),file=wire,flush=True)
    except Exception as e:
        traceback.print_exc(file=sys.stderr)
        print(json.dumps({'ok':False,'error':str(e)}),file=wire,flush=True)
