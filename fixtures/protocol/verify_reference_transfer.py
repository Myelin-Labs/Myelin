# Independent postcard field encoder and BIP340 verification. BLAKE3 is checked
# by the Rust golden-vector test; this script does not implement that primitive.
import hashlib,json
from pathlib import Path
from cryptography.hazmat.primitives.asymmetric import ec
v=json.loads(Path(__file__).with_name('reference-transfer-v1.json').read_text())
def raw(a): return bytes(a)
def uint(n):
 b=bytearray()
 while n>=128: b.append((n&127)|128); n>>=7
 b.append(n); return bytes(b)
def read_uint(b,i):
 n=s=0
 while True:
  x=b[i]; i+=1; n|=(x&127)<<s
  if x<128:return n,i
  s+=7
P=0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f
N=0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141
G=(0x79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798,0x483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8)
def add(a,b):
 if a is None:return b
 if b is None:return a
 x,y=a;u,w=b
 if x==u and (y+w)%P==0:return None
 m=((3*x*x)*pow(2*y,-1,P) if a==b else (w-y)*pow(u-x,-1,P))%P
 z=(m*m-x-u)%P;return z,(m*(x-z)-y)%P
def mul(k,a):
 r=None
 while k:
  if k&1:r=add(r,a)
  a=add(a,a);k>>=1
 return r
def verify(pk,msg,sig):
 x=int.from_bytes(pk,'big'); y=pow((x*x*x+7)%P,(P+1)//4,P)
 assert y*y%P==(x*x*x+7)%P
 if y%2:y=P-y
 r=int.from_bytes(sig[:32],'big');s=int.from_bytes(sig[32:],'big');assert r<P and s<N
 tag=hashlib.sha256(b'BIP0340/challenge').digest()
 e=int.from_bytes(hashlib.sha256(tag+tag+sig[:32]+pk+msg).digest(),'big')%N
 R=add(mul(s,G),mul((N-e)%N,(x,y)));assert R is not None and R[0]==r and R[1]%2==0
for vector in v['vectors']:
 p=vector['payload']; target=p['target']['Session']; src=p['source']; kind=0 if p['kind']=='scarce' else 1
 owner=ec.derive_private_key(int.from_bytes(bytes([3])*32,'big'),ec.SECP256K1()).public_key().public_numbers().x.to_bytes(32,'big')
 recipient=ec.derive_private_key(int.from_bytes(bytes([4])*32,'big'),ec.SECP256K1()).public_key().public_numbers().x.to_bytes(32,'big')
 assert owner==raw(p['owner']) and recipient==raw(p['recipient'])
 body=(raw(p['deployment_id'])+raw(p['uniqueness_domain'])+uint(kind)+raw(p['intake_schema'])+
       b''.join(raw(src[k]) for k in ['session_id','genesis_commitment','application_profile_commitment','consensus_module_commitment','effects_policy_hash'])+
       raw(p['asset']['namespace'])+raw(p['asset']['object'])+owner+recipient+uint(p['source_version'])+uint(p['amount'])+
       uint(len(p['components']))+raw(p['components'])+raw(p['nonce'])+uint(0)+raw(target['session_id'])+raw(target['genesis_commitment']))
 encoded=bytes.fromhex(vector['payload_hex']);assert encoded[:6]==b'MTRP\x01\x00' and int.from_bytes(encoded[6:14],'little')==len(body) and encoded[46:]==body
 auth=bytes.fromhex(vector['authorization_hex']);assert auth[:6]==b'MTRA\x01\x00'
 a=auth[46:]; version,offset=read_uint(a,64);nonce=a[offset:offset+32];size,sig_offset=read_uint(a,offset+32);sig=a[sig_offset:]
 assert owner==a[:32] and version==p['source_version'] and nonce==raw(p['nonce']) and size==64 and len(sig)==64
 scope=a[:offset+32]
 pre=(b'myelin:session-effect-handoff-authorization:v1'+raw(src['genesis_commitment'])+scope+uint(0)+raw(target['session_id'])+uint(100)+uint(len(encoded))+encoded+b'\x00')
 assert pre.hex()==vector['source_authorization_preimage_hex']
 verify(owner,bytes.fromhex(vector['transfer_id']),sig)
print('2 reference vectors: independent field encoding and BIP340 signatures verified')
