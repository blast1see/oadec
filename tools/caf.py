import numpy as np, struct
def read_caf(path):
    f=open(path,'rb'); assert f.read(4)==b'caff'; f.read(4)
    fmt=None; data=None
    while True:
        h=f.read(12)
        if len(h)<12: break
        ct=h[:4]; sz=struct.unpack('>q',h[4:])[0]
        if ct==b'desc':
            b=f.read(32)
            sr=struct.unpack('>d',b[:8])[0]; fid=b[8:12]
            flags,bpp,fpp,cpf,bits=struct.unpack('>IIIII',b[12:32])
            fmt=dict(sr=sr,fid=fid,flags=flags,bytes_per_packet=bpp,frames_per_packet=fpp,ch=cpf,bits=bits)
        elif ct==b'data':
            edit=f.read(4)
            n=sz-4 if sz>0 else None
            raw=f.read(n) if n else f.read()
            data=raw
        else:
            f.seek(sz if sz>0 else 0,1)
    ch=fmt['ch']; bits=fmt['bits']
    if bits==24:
        a=np.frombuffer(data,dtype=np.uint8)
        n=len(a)//3
        a=a[:n*3].reshape(n,3).astype(np.int32)
        v=(a[:,0]<<16)|(a[:,1]<<8)|a[:,2]          # big-endian
        v=np.where(v>=1<<23, v-(1<<24), v).astype(np.float64)/(1<<23)
        return v[:len(v)//ch*ch].reshape(-1,ch), fmt
    raise SystemExit('unsupported bits %d'%bits)
