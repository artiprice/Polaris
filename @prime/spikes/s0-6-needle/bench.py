import time,json,os,needle,psutil
p=psutil.Process()
def pc(x,q): x=sorted(x); return x[min(len(x)-1,int(q*len(x)))]
tools=json.load(open('tools.json'))
t=time.perf_counter(); a=needle.Needle(tools=tools); l0=time.perf_counter()-t
t=time.perf_counter(); r=a.complete("read the file src/main.rs"); first=time.perf_counter()-t
print("init s",l0,"first call s",first,"rss MB",p.memory_info().rss/1e6)
print("SAMPLE",json.dumps(r))
qs=["read the file src/main.rs","run the tests for crate polaris-core","search for TODO in the code","open Cargo.toml","test the crate net-utils","find where parse_args is defined","what is the weather in Paris"]
L=[];out=[]
for i in range(50):
    t=time.perf_counter(); r=a.complete(qs[i%7]); L.append((time.perf_counter()-t)*1000)
    if i<7: out.append((qs[i],r.get('function_calls')))
print("TOOL ms p50 %.1f p95 %.1f"%(pc(L,.5),pc(L,.95)))
for o in out: print(o)
E=[]
S=["The quick brown fox number %d jumps over the lazy dog near the river bank."%i for i in range(100)]
S=[s+" tokio runtime panics" if i%2 else s for i,s in enumerate(S)]
v=a.embed(S[0]); print("EMBED dim",len(v), type(v))
L=[]
for s in S:
    t=time.perf_counter(); a.embed(s); L.append((time.perf_counter()-t)*1000)
print("EMBED ms p50 %.1f p95 %.1f"%(pc(L,.5),pc(L,.95)))
print("peak rss MB",p.memory_info().peak_wset/1e6 if hasattr(p.memory_info(),'peak_wset') else p.memory_info().rss/1e6)
