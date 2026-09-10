"""Per-object comparison of oadec objects against Dolby's object decoder."""
import numpy as np, sys, json, os
sys.path.insert(0,os.path.dirname(os.path.abspath(__file__)))
from caf import read_caf
def corr_mat(A):
    A=A-A.mean(0); n=np.sqrt((A**2).sum(0)); n[n==0]=1
    return (A.T@A)/np.outer(n,n)
def run(name, caf_path, dolby_path, nch=16):
    X,_=read_caf(caf_path)
    D=np.fromfile(dolby_path,dtype='<f4')
    D=D[:len(D)//nch*nch].reshape(-1,nch).astype(np.float64)
    X=X.astype(np.float64)
    # lag search on the loudest object
    loud=int(np.argmax(np.sqrt((X**2).mean(0))[1:]))+1
    s=min(300000,len(X)//3); N=min(48000*4, len(X)-s-3000)
    a=X[s:s+N,loud]; best=(0,-2)
    for lag in range(-3000,3001,1):
        if s+lag<0 or s+lag+N>len(D): continue
        b=D[s+lag:s+lag+N,loud]
        d=np.sqrt((a**2).sum()*(b**2).sum())
        r=0.0 if d==0 else float((a*b).sum()/d)
        if r>best[1]: best=(lag,r)
    lag,lagr=best
    s2=200000; e2=min(len(X), len(D)-max(lag,0), s2+48000*25)
    A=X[s2:e2]; B=D[s2+lag:e2+lag]
    iu=np.triu_indices(nch,1)
    Mo=corr_mat(A); Md=corr_mat(B)
    per=[]
    for i in range(nch):
        x=A[:,i]; y=B[:,i]; d=x-y
        err=float(np.sqrt((d**2).mean())); sig=float(np.sqrt((y**2).mean()))
        sdr=None if sig==0 else (float('inf') if err==0 else round(20*np.log10(sig/err),2))
        rr=float((x*y).sum()/max(1e-30,np.sqrt((x**2).sum()*(y**2).sum())))
        per.append({'obj':i,'r':round(rr,6),'sdr_db':sdr,
                    'rms_ours':round(float(np.sqrt((x**2).mean())),6),
                    'rms_dolby':round(sig,6),
                    'silence_pct':round(float((np.abs(x)<1e-6).mean()*100),2)})
    fin=[p['sdr_db'] for p in per[1:] if p['sdr_db'] not in (None,) and np.isfinite(p['sdr_db'])]
    return {'name':name,'lag_samples':lag,'lag_corr':round(lagr,6),
            'compared_samples':int(e2-s2),
            'objects_worst_sdr_db':round(min(fin),2) if fin else None,
            'objects_median_sdr_db':round(float(np.median(fin)),2) if fin else None,
            'lfe_sdr_db':per[0]['sdr_db'],
            'max_obj_vs_obj_corr_ours':round(float(np.abs(Mo[iu]).max()),4),
            'max_obj_vs_obj_corr_dolby':round(float(np.abs(Md[iu]).max()),4),
            'mean_obj_corr_ours':round(float(np.abs(Mo[iu]).mean()),4),
            'mean_obj_corr_dolby':round(float(np.abs(Md[iu]).mean()),4),
            'corr_structure_max_abs_delta':round(float(np.abs(Mo[iu]-Md[iu]).max()),4),
            'pairs_over_0_9_ours':int((np.abs(Mo[iu])>0.9).sum()),
            'pairs_over_0_9_dolby':int((np.abs(Md[iu])>0.9).sum()),
            'per_object':per}
if __name__=='__main__':
    print(json.dumps(run(sys.argv[1],sys.argv[2],sys.argv[3]),indent=1))
