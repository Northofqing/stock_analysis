"""Protocol tests use separate copied tools and fake compiler/Cargo executables.

No real Cargo is invoked. No fixture has an issuance path: all receipts remain
RecordingOnly, and production CLI has no test-policy or inventory-path option.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

TOOL = Path(__file__).with_name("replay_build_owner_v1.py")
SPEC = importlib.util.spec_from_file_location("record_owner", TOOL)
owner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(owner)
PYTHON = str(Path(sys.executable).resolve())

# Exact ordered lint tokens from immutable record2, copied as data; no live-session dependency.
OBSERVED_LINTS = {'dep': ['--allow=unexpected_cfgs'],
 'build_script_build': ['--allow=clippy::used_underscore_binding',
                        '--allow=unused_qualifications',
                        '--warn=clippy::unnecessary_semicolon',
                        '--allow=clippy::unnecessary_cast',
                        '--allow=clippy::uninlined_format_args',
                        '--warn=clippy::ptr_as_ptr',
                        '--allow=clippy::non_minimal_cfg',
                        '--allow=clippy::missing_safety_doc',
                        '--warn=clippy::map_unwrap_or',
                        '--warn=clippy::manual_assert',
                        '--allow=clippy::identity_op',
                        '--warn=clippy::explicit_iter_loop',
                        '--allow=clippy::expl_impl_clone_on_copy'],
 'stock_analysis': ['--warn=clippy::unused_trait_names',
                    '--warn=unreachable_pub',
                    '--warn=unnameable_types',
                    '--warn=unexpected_cfgs',
                    '--warn=clippy::undocumented_unsafe_blocks',
                    '--warn=clippy::transmute_undefined_repr',
                    '--warn=clippy::trailing_empty_array',
                    '--warn=single_use_lifetimes',
                    '--warn=rust_2018_idioms',
                    '--warn=clippy::pedantic',
                    '--warn=non_ascii_idents',
                    '--warn=clippy::inline_asm_x86_att_syntax',
                    '--warn=improper_ctypes_definitions',
                    '--warn=improper_ctypes',
                    '--warn=deprecated_safe',
                    '--warn=clippy::default_union_representation',
                    '--warn=clippy::as_underscore',
                    '--warn=clippy::as_ptr_cast_mut',
                    '--warn=clippy::all',
                    '--allow=clippy::type_complexity',
                    '--allow=clippy::too_many_lines',
                    '--allow=clippy::too_many_arguments',
                    '--allow=clippy::struct_field_names',
                    '--allow=clippy::struct_excessive_bools',
                    '--allow=clippy::single_match_else',
                    '--allow=clippy::single_match',
                    '--allow=clippy::similar_names',
                    '--allow=clippy::range_plus_one',
                    '--allow=clippy::nonminimal_bool',
                    '--allow=clippy::naive_bytecount',
                    '--allow=clippy::module_name_repetitions',
                    '--allow=clippy::missing_errors_doc',
                    '--allow=clippy::manual_range_contains',
                    '--allow=clippy::manual_assert',
                    '--allow=clippy::incompatible_msrv',
                    '--allow=clippy::float_cmp',
                    '--allow=clippy::doc_markdown',
                    '--allow=clippy::declare_interior_mutable_const',
                    '--allow=clippy::collapsible_match',
                    '--allow=clippy::cast_lossless',
                    '--allow=clippy::borrow_as_ptr',
                    '--allow=clippy::bool_assert_comparison']}

LINT_MUTATIONS = {'empty': ['--allow='],
 'namespace': ['--warn=other::lint'],
 'nested_namespace': ['--allow=clippy::more::lint'],
 'space': ['--allow=unused name'],
 'equals': ['--warn=unused=value'],
 'path': ['--warn=../unused'],
 'comma': ['--allow=unused,dead_code'],
 'newline': ['--allow=unused\n'],
 'separated': ['--allow', 'unused'],
 'deny': ['--deny=unused=value'],
 'deny_empty': ['--deny='],
 'deny_space': ['--deny=unused name'],
 'deny_namespace': ['--deny=other::unused'],
 'deny_comma': ['--deny=unused,dead_code'],
 'deny_separated': ['--deny', 'unused'],
 'hyphen_unknown': ['--warn=clippy::unnecessary-wraps-extra'],
 'hyphen_path': ['--warn=clippy::or-fun-call/'],
 'hyphen_prefix': ['--allow=clippy::branches-sharing-code=other'],
 'hyphen_space': ['--deny=clippy::alloc-instead-of-core other'],
 'forbid': ['--forbid=unused'],
 'force_warn': ['--force-warn=unused'],
 'response': ['@response'],
 'unknown_codegen': ['-C', 'TEST_CODE_unknown=yes'],
 'unstable': ['-Zrandomize-layout']}

WRITEABLE_LINTS = ['--warn=clippy::wildcard_dependencies', '--warn=clippy::useless_transmute', '--warn=unused_qualifications', '--warn=unused_macro_rules', '--warn=unused_lifetimes', '--warn=clippy::unnecessary-wraps', '--warn=unexpected_cfgs', '--deny=clippy::trivially_copy_pass_by_ref', '--deny=trivial_numeric_casts', '--warn=clippy::transmutes_expressible_as_ptr_casts', '--warn=clippy::transmute_undefined_repr', '--warn=clippy::transmute_ptr_to_ref', '--warn=clippy::transmute_ptr_to_ptr', '--warn=clippy::transmute_int_to_non_zero', '--warn=clippy::transmute_int_to_bool', '--warn=clippy::transmute_bytes_to_str', '--warn=clippy::todo', '--warn=clippy::same_functions_in_if_condition', '--warn=clippy::or-fun-call', '--warn=clippy::negative_feature_names', '--warn=clippy::missing_transmute_annotations', '--warn=clippy::missing_fields_in_debug', '--deny=missing_debug_implementations', '--warn=clippy::mismatching_type_param_order', '--warn=clippy::large_stack_arrays', '--warn=clippy::infinite_loop', '--warn=clippy::fn_to_numeric_cast_any', '--deny=clippy::exhaustive_structs', '--deny=clippy::exhaustive_enums', '--warn=clippy::doc_markdown', '--warn=clippy::debug_assert_with_mut_call', '--warn=clippy::dbg_macro', '--warn=clippy::crosspointer_transmute', '--warn=clippy::collection_is_never_read', '--warn=clippy::branches-sharing-code', '--warn=clippy::alloc-instead-of-core']

JOBSERVER_MUTATIONS = (
    "mismatch", "reversed", "duplicate", "stdio", "negative", "overflow", "noncanonical",
    "closed", "regular", "empty", "fifo", "extra", "whitespace", "makeflags", "mflags",
)

AUTOCFG_BUILDER = r'''
import json,os,pathlib,subprocess,sys
out=pathlib.Path(os.environ['OUT_DIR']);out.mkdir(parents=True,exist_ok=True)
case=os.environ['FIXTURE_AUTOCFG_CASE'];cwd=os.environ.get('FIXTURE_AUTOCFG_CWD',os.environ['CARGO_MANIFEST_DIR'])
base=[os.environ['RUSTC_WRAPPER'],os.environ['RUSTC']]
version=subprocess.run(base+['--version','--verbose'],cwd=cwd,stdout=subprocess.PIPE)
if version.returncode: sys.exit(version.returncode)
bodies=[b'',b'#![no_std]',b'pub fn probe() { let _ = 1f64.total_cmp(&2f64); }']
count=0
prefix='0123456789abcdef'
def probe(body):
    global count
    index=count;count+=1
    name='autocfg_'+prefix+'_'+str(index)
    if case=='bad_index' and index==0:name='autocfg_'+prefix+'_3'
    if case=='bad_hex' and index==0:name='autocfg_TEST_CODE_0'
    if case=='mixed_prefix' and index==1:name='autocfg_fedcba9876543210_1'
    if case=='duplicate_index' and index==1:name='autocfg_'+prefix+'_0';body=b''
    command=base+['--crate-name',name,'--crate-type=lib','--out-dir',str(out),'--emit=llvm-ir','--target','x86_64-apple-darwin','-']
    if index==0:
        if case=='body':body=b'fn main(){}'
        if case=='newline':body=b'\n'
        if case=='nul':body=b'\0'
        if case=='oversize':body=b'X'*61
        if case=='extra':command.append('--edition=2021')
        if case=='native':command.append('-lTEST_CODE')
        if case=='target_arg':command[-2]='TEST_CODE'
        if case=='emit':command[-4]='--emit=metadata'
    result=subprocess.run(command,input=body,cwd=cwd,stdout=subprocess.PIPE)
    with open(os.environ['FIXTURE_AUTOCFG_STATUS'],'a') as log:log.write(json.dumps({'name':name,'body_hex':body.hex(),'code':result.returncode})+'\n')
    if result.returncode==0:
        try:(out/(name+'.ll')).unlink()
        except FileNotFoundError:pass
    if result.returncode not in (0,1):sys.exit(result.returncode)
    return result.returncode
std=probe(bodies[0])
if std==0:
    if case=='wrong_branch':probe(bodies[1])
    else:probe(bodies[2])
else:
    no_std=probe(bodies[1])
    probe((b'#![no_std]\n' if no_std==0 else b'')+bodies[2])
(out/'generated.rs').write_text('pub const TEST_CODE_AUTOCFG: u8 = 1;')
'''

RUSTIX_BUILDER = r'''
import fcntl,json,os,pathlib,subprocess,sys
bodies=(b'const unsafe fn foo(p: *const u8) -> isize { p.offset_from(p) }\n', b"fn a(x: &core::num::NonZeroI32, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { core::fmt::LowerExp::fmt(x, f) }\n", b'#[diagnostic::on_unimplemented()] trait Foo {}\n')
case=os.environ['FIXTURE_RUSTIX_CASE'];out=pathlib.Path(os.environ['OUT_DIR']);out.mkdir(parents=True,exist_ok=True)
order=[0,1,2]
if case=='reorder':order=[1,0,2]
if case=='duplicate':order=[0,0,2]
if case=='skip':order=[0,2]
for number in order:
    if number==1 and case=='prestate':(out/'rustix_test_can_compile').write_bytes(b'TEST_CODE_CHANGED')
    body=bodies[number]
    command=[os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],'--crate-type=rlib','--emit=metadata','--target','x86_64-apple-darwin','-o',str(out/'rustix_test_can_compile'),'-']
    if number==0:
        if case=='body':body=b'fn main() {}\n'
        if case=='empty':body=b''
        if case=='no_lf':body=body[:-1]
        if case=='extra_lf':body+=b'\n'
        if case=='nul':body+=b'\0'
        if case=='overflow':body=b'X'*123
        if case=='extra':command.append('--edition=2021')
        if case=='target_arg':command[5]='TEST_CODE'
        if case=='emit':command[3]='--emit=link'
        if case=='output':command[7]=str(out/'other')
        if case=='output_alias':command[7]=str(out)+'/./rustix_test_can_compile'
        if case=='native':command+=['-l','framework=SystemConfiguration']
    env=dict(os.environ)
    read,write=os.pipe();canary=fcntl.fcntl(read,fcntl.F_DUPFD,200)
    try:
        os.write(write,b'J')
        env.update(CARGO_MAKEFLAGS=f'-j --jobserver-fds={read},{write} --jobserver-auth={read},{write}',FIXTURE_RUSTIX_FDS=f'{read},{write},{canary}')
        result=subprocess.run(command,input=body,cwd=os.environ.get('FIXTURE_RUSTIX_CWD',os.environ['CARGO_MANIFEST_DIR']),env=env,pass_fds=(read,write,canary),stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    finally:
        for fd in (read,write,canary):os.close(fd)
    with open(out/'statuses.jsonl','a') as log:log.write(json.dumps({'index':number,'code':result.returncode,'stderr_hex':result.stderr.hex()})+'\n')
    if result.returncode not in (0,1):sys.exit(result.returncode)
'''

FAKE_RUSTC = r'''
import errno, fcntl, json, os, pathlib, stat, sys
AUTOCFG_BUILDER=__AUTOCFG_BUILDER__
RUSTIX_BUILDER=__RUSTIX_BUILDER__
args=sys.argv[1:]
hits=pathlib.Path(os.environ['FIXTURE_HIT_ROOT']);hits.mkdir(parents=True,exist_ok=True)
if args==['-vV']:
    (hits/'probe').write_text('entered');print('TEST_CODE_version_probe');sys.exit(0)
def value(key):
    values=[arg.split('=',1)[1] for arg in args if arg.startswith(key+'=')]
    return values[0] if values else args[args.index(key)+1]
if args==['--version']:
    (hits/'nested-version').write_text('entered');print('TEST_CODE_nested_version')
    sys.exit(int(os.environ.get('FIXTURE_VERSION_EXIT','0')))

if args==['--version','--verbose']:
    (hits/'autocfg-version').write_text('entered')
    print('rustc TEST_CODE\nrelease: 1.95.0')
    sys.exit(int(os.environ.get('FIXTURE_AUTOCFG_VERSION_EXIT','0')))
if args and args[-1]=='-' and '--emit=llvm-ir' in args:
    name=value('--crate-name');index=int(name.rsplit('_',1)[1]);body=sys.stdin.buffer.read()
    assert stat.S_ISFIFO(os.fstat(0).st_mode)
    (hits/('autocfg-'+str(index))).write_bytes(body)
    codes=json.loads(os.environ.get('FIXTURE_AUTOCFG_CODES','[0,0,0]'));code=codes[index]
    out=pathlib.Path(value('--out-dir'));out.mkdir(parents=True,exist_ok=True);output=out/(name+'.ll')
    if not os.environ.get('FIXTURE_AUTOCFG_MISSING') and (code==0 or os.environ.get('FIXTURE_AUTOCFG_FAILED_OUTPUT')):
        output.write_bytes(b'TEST_CODE_LL:'+name.encode()+b':'+body)
    if os.environ.get('FIXTURE_AUTOCFG_SYMLINK'):
        if output.exists():output.unlink()
        output.symlink_to(pathlib.Path(os.environ['CARGO_MANIFEST_DIR'])/'build.rs')
    print('X'*70000)
    print(json.dumps({'fixture_argv':args,'stdin_hex':body.hex(),'pipe':True}),file=sys.stderr)
    if os.environ.get('FIXTURE_AUTOCFG_SIGNAL'):os.kill(os.getpid(),9)
    sys.exit(code)


if args and args[-1]=='-' and '--emit=metadata' in args:
    body=sys.stdin.buffer.read();assert stat.S_ISFIFO(os.fstat(0).st_mode)
    bodies=(b'const unsafe fn foo(p: *const u8) -> isize { p.offset_from(p) }\n', b"fn a(x: &core::num::NonZeroI32, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { core::fmt::LowerExp::fmt(x, f) }\n", b'#[diagnostic::on_unimplemented()] trait Foo {}\n');index=bodies.index(body)
    (hits/('rustix-'+str(index))).write_bytes(body)
    read,write,canary=map(int,os.environ['FIXTURE_RUSTIX_FDS'].split(','))
    assert stat.S_ISFIFO(os.fstat(read).st_mode) and stat.S_ISFIFO(os.fstat(write).st_mode)
    try:os.fstat(canary)
    except OSError as e:assert e.errno==errno.EBADF
    else:raise AssertionError('canary leaked')
    assert os.read(read,1)==b'J';os.write(write,b'J')
    (hits/('rustix-fds-'+str(index))).write_text('same pair and closed canary')
    code=json.loads(os.environ['FIXTURE_RUSTIX_CODES'])[index];out=pathlib.Path(value('-o'))
    case=os.environ['FIXTURE_RUSTIX_CASE']
    if code==0 and case!='missing_output':out.write_bytes(b'TEST_CODE_METADATA:'+str(index).encode()+b':'+body)
    if code==1 and case=='remove_failed' and out.exists():out.unlink()
    if case=='symlink':
        if out.exists():out.unlink()
        out.symlink_to(pathlib.Path(os.environ['CARGO_MANIFEST_DIR'])/'build.rs')
    print('X'*70000);print('Y'*70000,file=sys.stderr)
    print(json.dumps({'fixture_argv':args,'stdin_hex':body.hex()}),file=sys.stderr)
    if case=='signal':os.kill(os.getpid(),9)
    sys.exit(code)

if '--cfg=procmacro2_build_probe' in args:
    source=next(pathlib.Path(a) for a in args if a.endswith('.rs'))
    (hits/('nested-'+source.stem)).write_text('entered')
    out=pathlib.Path(value('--out-dir'));out.mkdir(parents=True,exist_ok=True)
    code=int(os.environ.get('FIXTURE_PROBE_EXIT','1' if source.stem=='proc_macro_span' else '0'))
    source_path=pathlib.Path('/TEST_CODE_escape.rs') if os.environ.get('FIXTURE_DEP_ESCAPE') else source.resolve()
    (out/'proc_macro2.d').write_text('out: '+str(source_path)+chr(10))
    if code==0: (out/'libproc_macro2.rmeta').write_bytes(b'TEST_CODE_METADATA:'+source.read_bytes())
    if os.environ.get('FIXTURE_OUTPUT_SYMLINK'):
        (out/'proc_macro2.d').unlink();(out/'proc_macro2.d').symlink_to(source.resolve())
    print(json.dumps({'fixture_argv':args}),file=sys.stderr)
    if os.environ.get('FIXTURE_PROBE_SIGNAL'): os.kill(os.getpid(),9)
    sys.exit(code)
(hits/('compile-'+value('--crate-name'))).write_text('entered')
if 'FIXTURE_JOBSERVER_CHECK' in os.environ:
    read,write=map(int,os.environ['FIXTURE_JOBSERVER_CHECK'].split(','))
    assert stat.S_ISFIFO(os.fstat(read).st_mode) and stat.S_ISFIFO(os.fstat(write).st_mode)
    assert fcntl.fcntl(read,fcntl.F_GETFL)&os.O_ACCMODE==os.O_RDONLY
    assert fcntl.fcntl(write,fcntl.F_GETFL)&os.O_ACCMODE==os.O_WRONLY
    canary=int(os.environ['FIXTURE_CLOSED_CANARY'])
    try: os.fstat(canary)
    except OSError as error: assert error.errno==errno.EBADF
    else: raise AssertionError('unrelated descriptor leaked to compiler')
    token=os.read(read,1);assert token==b'J';assert os.write(write,token)==1
    (hits.parent/('jobserver-observed-'+value('--crate-name')+'.json')).write_text(json.dumps(
        {'pair':[read,write],'canary':canary,'canary_closed':True,'token_hex':token.hex()}))
name=value('--crate-name'); out=pathlib.Path(value('--out-dir')); out.mkdir(parents=True,exist_ok=True)
source=next(pathlib.Path(a) for a in reversed(args) if a.endswith('.rs')); body=source.read_bytes()
if name=='build_script_build':
    executable=out/name
    executable.write_text("#!"+sys.executable+chr(10)+"""import json,os,pathlib,shutil,subprocess,sys
p=pathlib.Path(os.environ['OUT_DIR']);p.mkdir(parents=True,exist_ok=True)
(p/'generated.rs').write_text('pub const FIXTURE: u8 = 1;')
(p/'private.rs').write_text('pub const TEST_CODE_PRIVATE: u8 = 1;')
for command in json.loads(os.environ.get('FIXTURE_NESTED_COMMANDS','[]')):
    (p/'probe').mkdir(exist_ok=True)
    result=subprocess.run(command,cwd=os.environ.get('FIXTURE_NESTED_CWD',os.environ['CARGO_MANIFEST_DIR']),stdout=subprocess.PIPE)
    with open(os.environ['FIXTURE_STATUS_LOG'],'a') as log: log.write(str(result.returncode)+'\\n')
    shutil.rmtree(p/'probe',ignore_errors=True)
    if command[-1]=='--version' and result.returncode: sys.exit(result.returncode)
    if command[-1]!='--version' and result.returncode not in (0,1): sys.exit(result.returncode)
""")
    if os.environ.get('FIXTURE_BUILD_AUTOCFG'):
        executable.write_text("#!"+sys.executable+chr(10)+AUTOCFG_BUILDER)
    if os.environ.get('FIXTURE_BUILD_RUSTIX'):
        executable.write_text('#!'+sys.executable+chr(10)+RUSTIX_BUILDER)
    executable.chmod(0o700)
else:
    for suffix in ['.rmeta','.rlib']:
        (out/('lib'+name+suffix)).write_bytes(name.encode()+b':'+body)
    for emit in value('--emit').split(','):
        if emit.startswith('link='):
            explicit=pathlib.Path(emit.split('=',1)[1]);explicit.parent.mkdir(parents=True,exist_ok=True)
            explicit.write_bytes((out/('lib'+name+'.rlib')).read_bytes())
inputs=[str(source)]
if name=='stock_analysis': inputs.append(str(pathlib.Path(os.environ['OUT_DIR'])/os.environ.get('FIXTURE_GENERATED_NAME','generated.rs')))
def escape(x): return x.replace(chr(92),chr(92)*2).replace(' ',chr(92)+' ').replace('#',chr(92)+'#').replace(':',chr(92)+':').replace('$','$$')
(out/(name+'.d')).write_text(escape(str(out/(name+'.rlib')))+': '+' '.join(map(escape,inputs))+chr(10))
print(json.dumps({'fixture_argv':args}),file=sys.stderr)
if os.environ.get('FIXTURE_FAIL')=='1': sys.exit(7)
'''

FAKE_CARGO = r'''
import contextlib, fcntl, json, os, pathlib, shutil, subprocess, sys
MODE=__MODE__
LINTS=__LINTS__
ARG_CASES=__ARG_CASES__
args=sys.argv[1:]
assert args[:5]==['build','--locked','--offline','--lib','--target']
assert args[5]=='x86_64-apple-darwin'
def value(key): return args[args.index(key)+1]
app=pathlib.Path(value('--manifest-path')).parent; session=app.parent; target=pathlib.Path(value('--target-dir'))
def emit(value): print(json.dumps(value),flush=True)
sysroot_lib=str(pathlib.Path(os.environ['RUSTC']).parent/'sysroot/lib')
assert os.environ['DYLD_FALLBACK_LIBRARY_PATH']==sysroot_lib
assert not any(k in os.environ for k in ('CARGO_MAKEFLAGS','MAKEFLAGS','MFLAGS'))
prefix=str(target/'debug/deps')
probe_env=dict(os.environ,FIXTURE_HIT_ROOT=str(session/'compiler-entry'))
if MODE=='probe_compile_path': probe_env['DYLD_FALLBACK_LIBRARY_PATH']=prefix+':'+sysroot_lib
probe=subprocess.run([os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],'-vV'],env=probe_env,stdout=subprocess.PIPE)
if probe.returncode: emit({'reason':'build-finished','success':False});sys.exit(probe.returncode)
@contextlib.contextmanager
def jobserver(env,name):
    if not MODE.startswith('jobserver_') or MODE=='jobserver_missing':
        yield env,();return
    read,write=os.pipe();owned=[read,write]
    try:
        canary_file=os.open(session/'fd-canary',os.O_CREAT|os.O_RDWR,0o600)
        try: canary=fcntl.fcntl(canary_file,fcntl.F_DUPFD,200)
        finally: os.close(canary_file)
        owned.append(canary)
        def flags(r,w): return f'-j --jobserver-fds={r},{w} --jobserver-auth={r},{w}'
        child=dict(env,CARGO_MAKEFLAGS=flags(read,write),FIXTURE_JOBSERVER_CHECK=f'{read},{write}',
                   FIXTURE_CLOSED_CANARY=str(canary))
        if MODE.startswith('jobserver_invalid:') and name=='dep':
            case=MODE.split(':',1)[1]
            closed=fcntl.fcntl(read,fcntl.F_DUPFD,100);os.close(closed)
            mutations={'mismatch':f'-j --jobserver-fds={read},{write} --jobserver-auth={write},{read}',
                       'reversed':flags(write,read),'duplicate':flags(read,read),'stdio':flags(1,write),
                       'negative':flags(-1,write),'overflow':flags(1<<100,write),
                       'noncanonical':flags('0'+str(read),write),'closed':flags(closed,write),
                       'regular':flags(canary,write),'empty':'','fifo':'-j --jobserver-auth=fifo:/TEST_CODE',
                       'extra':flags(read,write)+' -j2','whitespace':' '+flags(read,write)}
            if case in mutations: child['CARGO_MAKEFLAGS']=mutations[case]
            elif case in ('makeflags','mflags'): child[case.upper()]=flags(read,write)
            else: raise AssertionError(case)
        os.write(write,b'J')
        (session/('jobserver-attempt-'+name+'.json')).write_text(json.dumps({
            'pair':[read,write],'canary':canary,'environment':{k:child[k] for k in
                ('CARGO_MAKEFLAGS','MAKEFLAGS','MFLAGS') if k in child}}))
        yield child,tuple(owned)
    finally:
        for fd in owned: os.close(fd)

def compile(name, root, manifest, kind, package, out, extra=(), env_extra=None):
    env=dict(os.environ,CARGO_MANIFEST_DIR=str(manifest),FIXTURE_HIT_ROOT=str(session/'compiler-entry'))
    env['DYLD_FALLBACK_LIBRARY_PATH']=prefix+':'+sysroot_lib
    if name=='dep':
        mutations={'loader_extra':prefix+':'+sysroot_lib+':/usr/local/lib',
                   'loader_reversed':sysroot_lib+':'+prefix,
                   'loader_foreign':str(session.parent/'pending-foreign/target/debug/deps')+':'+sysroot_lib,
                   'loader_empty':'', 'loader_empty_component':prefix+'::'+sysroot_lib,
                   'loader_duplicate':prefix+':'+prefix+':'+sysroot_lib}
        if MODE in mutations: env['DYLD_FALLBACK_LIBRARY_PATH']=mutations[MODE]
        if MODE=='loader_missing': del env['DYLD_FALLBACK_LIBRARY_PATH']
        if MODE.startswith('forbidden_'): env[MODE[len('forbidden_'):]]='/TEST_CODE_FOREIGN'
    env.update(env_extra or {})
    command=[os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],'--crate-name',name,'--edition','2021','--crate-type',kind,'--emit','dep-info,link' if kind=='bin' else 'dep-info,metadata,link','--out-dir',str(out),*extra,str(root)]
    if name=='stock_analysis' and env.get('FIXTURE_ORDINARY_REUSE'):
        command[command.index('--emit')+1]='dep-info,link='+env['FIXTURE_ORDINARY_REUSE']
    if MODE=='wrong_sysroot' and name=='dep': command[2:2]=['--sysroot','/TEST_CODE_wrong_sysroot']
    if MODE=='writeable_lints': command[2:2]=__WRITEABLE_LINTS__
    if MODE=='lint_equals':
        command[2:2]=LINTS[name]+(['--target','x86_64-apple-darwin'] if name=='stock_analysis' else [])
    if MODE.startswith('lint_invalid:') and name=='dep': command[2:2]=ARG_CASES[MODE.split(':',1)[1]]
    with jobserver(env,name) as (child,descriptors):
        result=subprocess.run(command,env=child,pass_fds=descriptors)
    if result.returncode: emit({'reason':'build-finished','success':False});sys.exit(result.returncode)
    if kind=='bin':
        alias=out/'build-script-build';shutil.copy2(out/name,alias);files=[str(alias)];executable=str(alias)
    else: files=[str(out/('lib'+name+'.rlib')),str(out/('lib'+name+'.rmeta'))];executable=None
    if name=='stock_analysis' and env.get('FIXTURE_ARTIFACT_ALIAS'):
        alias=pathlib.Path(env['FIXTURE_ARTIFACT_ALIAS']);alias.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(out/('lib'+name+'.rlib'),alias)
        files=[env['FIXTURE_ARTIFACT_ALIAS']]
    if MODE.startswith('nested:') and kind=='bin':
        event_executable=None
        if MODE=='nested:nonnull_mismatch': event_executable=str(out/'TEST_CODE_wrong')
        if MODE=='nested:alias_bytes': alias.write_bytes(b'TEST_CODE_CHANGED')
        if MODE=='nested:multiple_filenames': files.append(str(out/name))
        if MODE=='nested:metadata_alias':
            files=[str(out/(name+'.d'))];event_executable=None
    else: event_executable=executable
    event={'reason':'compiler-artifact','package_id':package,'target':{'src_path':str(root),'kind':['custom-build'] if kind=='bin' else ['lib'],'name':name,'crate_types':[kind]},'filenames':files,'executable':event_executable,'fresh':False}
    if MODE=='nested:wrong_package' and kind=='bin': event['package_id']='TEST_CODE_WRONG'
    emit(event)
    if MODE=='nested:duplicate_producer' and kind=='bin': emit(event)
    return str(out/name) if MODE=="nested:alias_bytes" and kind=="bin" else executable
compile('dep',session/'vendor/dep/src/lib.rs',session/'vendor/dep','rlib','TEST_CODE_dep',target/'deps',env_extra={'FIXTURE_FAIL':'1'} if MODE=='compiler_fails' else {})
script=compile('build_script_build',app/'build.rs',app,'bin','TEST_CODE_app',target/'build-bin')
out=target/'build-output';subprocess.run([script],env=dict(os.environ,OUT_DIR=str(out)),check=True)
if MODE!='missing_producer': emit({'reason':'build-script-executed','package_id':'TEST_CODE_app','out_dir':str(out),'cfgs':[],'env':[],'linked_libs':[],'linked_paths':[]})
compile('stock_analysis',app/'src/lib.rs',app,'rlib','TEST_CODE_app',target/'deps',extra=['--extern','dep='+str(target/'deps/libdep.rmeta')],env_extra={'OUT_DIR':str(out)})
if MODE=='output_drift': (target/'deps/libdep.rmeta').write_bytes(b'TEST_CODE_CHANGED')
if MODE=='source_drift': (app/'src/lib.rs').chmod(0o644);(app/'src/lib.rs').write_text('changed')
if MODE=='unknown_event': emit({'reason':'TEST_CODE_unknown','producer':'do not guess'})
emit({'reason':'build-finished','success':True})
'''


NESTED_CARGO_FLOW = r'''
if MODE.startswith('nested:'):
    case=MODE.split(':',1)[1]
    hostlib=str(pathlib.Path(sysroot_lib)/'rustlib/x86_64-apple-darwin/lib')
    loader=':'.join([str(target/'debug'),str(target/'debug/deps'),hostlib,sysroot_lib])
    origin_records=[]
    for package_name,version in [('libc','0.2.184'),('proc-macro2','1.0.106')]:
        package='registry+https://github.com/rust-lang/crates.io-index#'+package_name+'@'+version
        root=session/'vendor'/package_name
        builder=compile('build_script_build',root/'build.rs',root,'bin',package,
                        target/'debug/build'/('TEST_CODE_builder_'+package_name))
        # Replace just-emitted ordinary fake event behavior through compile's branch below.
        forms=['debug','x86_64-apple-darwin/debug'] if package_name=='libc' else ['debug']
        for form in forms:
            out=target/form/'build'/('TEST_CODE_output_'+package_name)/'out';out.mkdir(parents=True,exist_ok=True)
            env=dict(os.environ,CARGO_MANIFEST_DIR=str(root),CARGO_PKG_NAME=package_name,
                     HOST='x86_64-apple-darwin',TARGET='x86_64-apple-darwin',OUT_DIR=str(out),
                     DYLD_FALLBACK_LIBRARY_PATH=loader,FIXTURE_HIT_ROOT=str(session/'compiler-entry'),
                     FIXTURE_STATUS_LOG=str(session/'nested-status.raw'),FIXTURE_NESTED_CWD=str(root))
            if package_name=='libc': commands=[[os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],'--version']]
            else:
                commands=[[os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],'--cfg=procmacro2_build_probe',
                           '--edition=2021','--crate-name=proc_macro2','--crate-type=lib','--cap-lints=allow',
                           '--emit=dep-info,metadata','--out-dir',str(out/'probe'),'src/probe/'+name+'.rs',
                           '--target','x86_64-apple-darwin'] for name in
                          ['proc_macro_span_location','proc_macro_span_file','proc_macro_span']]
            # Injection cases target the first nested call. The already-built generator is not a compiler hit for that call.
            if package_name=='libc' and form=='debug':
                mutations={'extra':loader+':/usr/local/lib','permuted':':'.join(reversed(loader.split(':'))),
                           'foreign':loader.replace(str(target/'debug'),'/TEST_CODE_FOREIGN',1),
                           'empty':'','empty_component':loader+':'}
                if case in mutations: env['DYLD_FALLBACK_LIBRARY_PATH']=mutations[case]
                if case in ('host','target','package','manifest','wrapper'):
                    key={'host':'HOST','target':'TARGET','package':'CARGO_PKG_NAME','manifest':'CARGO_MANIFEST_DIR','wrapper':'RUSTC_WRAPPER'}[case]
                    env[key]='/TEST_CODE_WRONG'
                if case=='outdir': env['OUT_DIR']=str(target/'TEST_CODE_arbitrary_out');pathlib.Path(env['OUT_DIR']).mkdir()
                if case=='cwd': env['FIXTURE_NESTED_CWD']=str(app)
                if case=='version_failure': env['FIXTURE_VERSION_EXIT']='1'
                if case in ('encoded','bootstrap'):
                    env['CARGO_ENCODED_RUSTFLAGS' if case=='encoded' else 'RUSTC_BOOTSTRAP']='TEST_CODE'
                if case=='direct_disguise': commands=[[os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],'-vV']]
            if package_name=='proc-macro2':
                if case=='unknown_probe': commands[0][-3]='src/probe/unknown.rs'
                if case=='source_substitution': commands[0][-3]='src/lib.rs'
                if case=='unknown_flag': commands[0].append('-ZTEST_CODE')
                if case=='exit2': env['FIXTURE_PROBE_EXIT']='2'
                if case=='signal': env['FIXTURE_PROBE_SIGNAL']='1'
                if case=='symlink': env['FIXTURE_OUTPUT_SYMLINK']='1'
                if case=='escaped_dep': env['FIXTURE_DEP_ESCAPE']='1'
            env['FIXTURE_NESTED_COMMANDS']=json.dumps(commands)
            result=subprocess.run([builder],env=env)
            if result.returncode: emit({'reason':'build-finished','success':False});sys.exit(result.returncode)
            event={'reason':'build-script-executed','package_id':package,'out_dir':str(out),'cfgs':[],
                   'env':[],'linked_libs':[],'linked_paths':[]}
            if case not in ('missing_origin','wrong_origin'): emit(event)
            if case=='duplicate_event': emit(event)
            if case=='wrong_origin':
                event=dict(event,out_dir=str(target/'TEST_CODE_wrong_origin'));pathlib.Path(event['out_dir']).mkdir(exist_ok=True);emit(event)
            origin_records.append((package,str(out)))
    if case in ('transient_extern','transient_consumed'):
        probe=target/'debug/build/TEST_CODE_output_proc-macro2/out/probe';probe.mkdir()
        (probe/'libproc_macro2.rmeta').write_bytes(b'TEST_CODE_RECREATED')
    # A normal library consumes actual generated private.rs from the Target execution of a Host builder.
    out=target/'x86_64-apple-darwin/debug/build/TEST_CODE_output_libc/out'
    generated_name='private.rs'
    if case=='transient_consumed': out=probe.parent;generated_name='probe/libproc_macro2.rmeta'
    final_env={'OUT_DIR':str(out),'FIXTURE_GENERATED_NAME':generated_name}
    if case in ('artifact_alias','artifact_alias_dot','ordinary_alias','unsupported_artifact_alias','unsupported_ordinary_alias','nontransient_alias'):
        reused=target/'debug/build/TEST_CODE_output_proc-macro2/out/probe/libproc_macro2.rmeta'
        alias=str(target/'TEST_CODE_legitimate_alias') if case=='nontransient_alias' else str(reused)
        if case=='artifact_alias_dot':alias=str(reused.parent)+'/./'+reused.name
        final_env['FIXTURE_ORDINARY_REUSE' if 'ordinary' in case else 'FIXTURE_ARTIFACT_ALIAS']=alias
    compile('stock_analysis',app/'src/lib.rs',app,'rlib','TEST_CODE_app',target/'deps',
            extra=['--target','x86_64-apple-darwin']+(['--extern','probe='+str(probe/'libproc_macro2.rmeta')] if case=='transient_extern' else []),env_extra=final_env)
    if case in ('snapshot_tamper','snapshot_missing'):
        snapshot=next((session/'invocations').glob('*/probe-output-*.raw'))
        if case=='snapshot_tamper': snapshot.write_bytes(b'TEST_CODE_CHANGED')
        else: snapshot.unlink()
    emit({'reason':'build-finished','success':True});sys.exit(0)
'''


AUTOCFG_CARGO_FLOW = r'''
if MODE.startswith('autocfg:'):
    case=MODE.split(':',1)[1]
    package='registry+https://github.com/rust-lang/crates.io-index#num-traits@0.2.19'
    helper_package='registry+https://github.com/rust-lang/crates.io-index#autocfg@1.5.0'
    root=session/'vendor/num-traits';helper=session/'vendor/autocfg'
    compile('autocfg',helper/'src/lib.rs',helper,'rlib',helper_package,target/'debug/deps')
    helper_path=target/'debug/deps/libautocfg.rlib'
    extras=[] if case=='missing_helper' else ['--extern','autocfg='+str(helper_path)]
    if case=='wrong_helper':
        compile('dep',session/'vendor/dep/src/lib.rs',session/'vendor/dep','rlib','TEST_CODE_dep',target/'deps')
        extras=['--extern','autocfg='+str(target/'deps/libdep.rlib')]
    builder=compile('build_script_build',root/'build.rs',root,'bin',package,target/'debug/build/TEST_CODE_numtraits_builder',extra=extras,env_extra={'FIXTURE_BUILD_AUTOCFG':'1'})
    out=target/'x86_64-apple-darwin/debug/build/TEST_CODE_numtraits_output/out';out.mkdir(parents=True)
    hostlib=str(pathlib.Path(sysroot_lib)/'rustlib/x86_64-apple-darwin/lib')
    loader=':'.join([str(target/'debug'),str(target/'debug/deps'),hostlib,sysroot_lib])
    env=dict(os.environ,CARGO_MANIFEST_DIR=str(root),CARGO_PKG_NAME='num-traits',HOST='x86_64-apple-darwin',TARGET='x86_64-apple-darwin',OUT_DIR=str(out),DYLD_FALLBACK_LIBRARY_PATH=loader,FIXTURE_HIT_ROOT=str(session/'compiler-entry'),FIXTURE_AUTOCFG_CASE=case,FIXTURE_AUTOCFG_CWD=str(root),FIXTURE_AUTOCFG_STATUS=str(session/'autocfg-status.jsonl'))
    codes={'nostd':[1,0,0],'neither':[1,1,1],'expr_unsupported':[0,1,0],'exit2':[2,0,0],
           'unsupported_artifact_alias':[0,1,0],'unsupported_ordinary_alias':[0,1,0]}
    env['FIXTURE_AUTOCFG_CODES']=json.dumps(codes.get(case,[0,0,0]))
    if case=='version_failure':env['FIXTURE_AUTOCFG_VERSION_EXIT']='1'
    if case=='signal':env['FIXTURE_AUTOCFG_SIGNAL']='1'
    if case=='symlink':env['FIXTURE_AUTOCFG_SYMLINK']='1'
    if case=='missing_output':env['FIXTURE_AUTOCFG_MISSING']='1'
    if case=='neither':env['FIXTURE_AUTOCFG_FAILED_OUTPUT']='1'
    if case=='loader':env['DYLD_FALLBACK_LIBRARY_PATH']=loader+':/TEST_CODE'
    if case in ('package','manifest','target_env','outdir'):
        key={'package':'CARGO_PKG_NAME','manifest':'CARGO_MANIFEST_DIR','target_env':'TARGET','outdir':'OUT_DIR'}[case]
        env[key]=str(target/'TEST_CODE_other') if case=='outdir' else 'TEST_CODE'
    if case=='cwd':env['FIXTURE_AUTOCFG_CWD']=str(app)
    result=subprocess.run([builder],env=env)
    if result.returncode:emit({'reason':'build-finished','success':False});sys.exit(result.returncode)
    event={'reason':'build-script-executed','package_id':package,'out_dir':str(out),'cfgs':[],'env':[],'linked_libs':[],'linked_paths':[]}
    if case!='missing_origin':emit(event)
    extra=['--target','x86_64-apple-darwin'];generated='generated.rs'
    if case in ('transient_extern','transient_consumed'):
        probe=out/'autocfg_0123456789abcdef_0.ll';probe.write_bytes(b'TEST_CODE_RECREATED')
        if case=='transient_extern':extra+=['--extern','probe='+str(probe)]
        else:generated=probe.name
    final_env={'OUT_DIR':str(out),'FIXTURE_GENERATED_NAME':generated}
    if case in ('artifact_alias','artifact_alias_dot','ordinary_alias','unsupported_artifact_alias','unsupported_ordinary_alias','nontransient_alias'):
        reused=out/('autocfg_0123456789abcdef_'+('1' if case.startswith('unsupported') else '0')+'.ll')
        alias=str(target/'TEST_CODE_legitimate_alias') if case=='nontransient_alias' else str(reused)
        if case=='artifact_alias_dot':alias=str(reused.parent)+'/./'+reused.name
        final_env['FIXTURE_ORDINARY_REUSE' if 'ordinary' in case else 'FIXTURE_ARTIFACT_ALIAS']=alias
    compile('stock_analysis',app/'src/lib.rs',app,'rlib','TEST_CODE_app',target/'deps',extra=extra,env_extra=final_env)
    if case in ('stdin_tamper','stdin_missing','stdin_meta','snapshot_tamper','snapshot_missing'):
        pattern='*/probe-output-*.raw' if case.startswith('snapshot') else '*/stdin.raw'
        path=next((session/'invocations').glob(pattern))
        if case.endswith('missing'):path.unlink()
        elif case=='stdin_meta':path.with_name('stdin.json').write_text('{}')
        else:path.write_bytes(b'TEST_CODE_CHANGED')
    emit({'reason':'build-finished','success':True});sys.exit(0)
'''

FIX5_CARGO_FLOW = r'''
if MODE.startswith('fix5:'):
    family,case=MODE.split(':')[1:]
    app_builder=compile('build_script_build',app/'build.rs',app,'bin','TEST_CODE_app',target/'app-builder')
    app_out=target/'app-out';subprocess.run([app_builder],env=dict(os.environ,OUT_DIR=str(app_out)),check=True)
    emit({'reason':'build-script-executed','package_id':'TEST_CODE_app','out_dir':str(app_out),'cfgs':[],'env':[],'linked_libs':[],'linked_paths':[]})
    name,version=('rustix','1.1.4') if family=='rustix' else ('system-configuration-sys','0.6.0')
    package='registry+https://github.com/rust-lang/crates.io-index#'+name+'@'+version
    root=session/'vendor'/name;out=target/'x86_64-apple-darwin/debug/build'/('TEST_CODE_'+name)/'out';out.mkdir(parents=True)
    cfgs=['feature="'+f+'"' for f in ['alloc','default','fs','std','stdio','termios']]
    extra=[a for c in cfgs for a in ('--cfg',c)] if family=='rustix' else []
    if case=='builder_features':extra=[]
    builder=compile('build_script_build',root/'build.rs',root,'bin',package,target/'debug/build'/('TEST_CODE_builder_'+name),extra=extra,env_extra={'FIXTURE_BUILD_RUSTIX':'1'} if family=='rustix' else {})
    native=['framework=SystemConfiguration'] if family=='framework' else []
    event={'reason':'build-script-executed','package_id':package,'out_dir':str(out),'cfgs':[],'env':[],'linked_libs':native,'linked_paths':[]}
    if family=='rustix':
        hostlib=str(pathlib.Path(sysroot_lib)/'rustlib/x86_64-apple-darwin/lib')
        loader=':'.join([str(target/'debug'),str(target/'debug/deps'),hostlib,sysroot_lib])
        codes=[int(c) for c in case[-3:]] if case.startswith('codes') else [0,0,0]
        if case=='exit2':codes=[2,0,0]
        if case=='remove_failed':codes=[0,1,0]
        if case.startswith('absent_'):codes=[1,1,1]
        env=dict(os.environ,CARGO_MANIFEST_DIR=str(root),CARGO_PKG_NAME=name,CARGO_PKG_VERSION=version,HOST='x86_64-apple-darwin',TARGET='x86_64-apple-darwin',OUT_DIR=str(out),DYLD_FALLBACK_LIBRARY_PATH=loader,FIXTURE_HIT_ROOT=str(session/'compiler-entry'),FIXTURE_RUSTIX_CASE=case,FIXTURE_RUSTIX_CODES=json.dumps(codes))
        env.update({'CARGO_FEATURE_'+f.upper():'1' for f in ['alloc','default','fs','std','stdio','termios']})
        env.update(CARGO_CFG_TARGET_ARCH='x86_64',CARGO_CFG_TARGET_OS='macos',CARGO_CFG_TARGET_ENDIAN='little',CARGO_CFG_TARGET_POINTER_WIDTH='64',CARGO_CFG_TARGET_ABI='',CARGO_CFG_TARGET_ENV='',CARGO_ENCODED_RUSTFLAGS='')
        mutations={'package':('CARGO_PKG_NAME','other'),'version':('CARGO_PKG_VERSION','1.1.5'),'target_env':('TARGET','other'),'std':('CARGO_FEATURE_STD','0'),'extra_feature':('CARGO_FEATURE_NET','1'),'encoded':('CARGO_ENCODED_RUSTFLAGS','TEST_CODE'),'loader':('DYLD_FALLBACK_LIBRARY_PATH',loader+':/other'),'outdir':('OUT_DIR',str(target/'other'))}
        if case in mutations:env.update([mutations[case]])
        if case=='outdir':pathlib.Path(env['OUT_DIR']).mkdir()
        if case=='cwd':env['FIXTURE_RUSTIX_CWD']=str(app)
        if case=='preexisting':(out/'rustix_test_can_compile').write_bytes(b'TEST_CODE_EXISTING')
        if case=='source':(root/'build.rs').chmod(0o644);(root/'build.rs').write_text('changed')
        result=subprocess.run([builder],env=env)
        if case=='source':(root/'build.rs').write_text('// TEST_CODE fixed5 generator\n')
        if result.returncode:emit({'reason':'build-finished','success':False});sys.exit(result.returncode)
        success_cfg=['static_assertions','lower_upper_exp_for_non_zero','rustc_diagnostics']
        event['cfgs']=[c for c,code in zip(success_cfg,codes) if code==0]+['libc','apple','bsd']
    else:
        subprocess.run([builder],env=dict(os.environ,OUT_DIR=str(out)),check=True)
    if case=='cfg':event['cfgs']=['TEST_CODE_WRONG']
    if case=='linked_libs':event['linked_libs']=['framework=Other']
    if case=='linked_paths':event['linked_paths']=['framework=/other']
    if case=='wrong_origin':event['package_id']='TEST_CODE_WRONG'
    if case!='missing_origin':emit(event)
    if case=='duplicate_origin':emit(event)
    if family=='framework':
        env=dict(os.environ,CARGO_MANIFEST_DIR=str(root),CARGO_PKG_NAME=name,CARGO_PKG_VERSION=version,OUT_DIR=str(out),DYLD_FALLBACK_LIBRARY_PATH=prefix+':'+sysroot_lib,FIXTURE_HIT_ROOT=str(session/'compiler-entry'))
        env_mutations={'package':('CARGO_PKG_NAME','other'),'version':('CARGO_PKG_VERSION','0.6.1'),'outdir':('OUT_DIR',str(target/'other'))}
        if case in env_mutations:env.update([env_mutations[case]])
        if case=='outdir':pathlib.Path(env['OUT_DIR']).mkdir()
        dest=target/'x86_64-apple-darwin/debug/deps';dest.mkdir(parents=True,exist_ok=True)
        command=[os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],'--crate-name','system_configuration_sys','--crate-type','lib','--emit=dep-info,metadata,link','--out-dir',str(dest),'--target','x86_64-apple-darwin',str(root/'src/lib.rs'),'-l','framework=SystemConfiguration']
        if case=='other_framework':command[-1]='framework=Other'
        if case=='modifier':command[-1]='framework:+bundle=SystemConfiguration'
        if case=='attached':command[-2:]=['-lframework=SystemConfiguration']
        if case=='duplicate_flag':command+=command[-2:]
        if case=='reordered':command=command[:-3]+command[-2:]+command[-3:-2]
        if case=='link_arg':command[-2:-2]=['-C','link-arg=-lOther']
        if case=='native_search':command[-2:-2]=['-L','native='+str(target)]
        if case=='output':command[command.index('--out-dir')+1]=str(target/'other')
        if case=='source':command[command.index(str(root/'src/lib.rs'))]=str(app/'src/lib.rs')
        result=subprocess.run(command,env=env,cwd=root)
        if result.returncode:emit({'reason':'build-finished','success':False});sys.exit(result.returncode)
        emit({'reason':'compiler-artifact','package_id':package,'target':{'src_path':str(root/'src/lib.rs'),'kind':['lib'],'name':'system_configuration_sys','crate_types':['lib']},'filenames':[str(dest/'libsystem_configuration_sys.rlib'),str(dest/'libsystem_configuration_sys.rmeta')],'executable':None,'fresh':False})
    final_env={'OUT_DIR':str(app_out)};extra=[];path=out/'rustix_test_can_compile'
    alias_case=case.removeprefix('absent_')
    if family=='rustix' and alias_case in ('ordinary_alias','artifact_alias','artifact_dot','nontransient_alias'):
        alias=str(target/'legitimate_alias') if case=='nontransient_alias' else str(path)
        if alias_case=='artifact_dot':alias=str(out)+'/./'+path.name
        final_env['FIXTURE_ORDINARY_REUSE' if alias_case=='ordinary_alias' else 'FIXTURE_ARTIFACT_ALIAS']=alias
    if family=='rustix' and case in ('extern','consumed'):
        if case=='extern':extra=['--extern','probe='+str(path)]
        else:final_env={'OUT_DIR':str(out),'FIXTURE_GENERATED_NAME':path.name}
    compile('stock_analysis',app/'src/lib.rs',app,'rlib','TEST_CODE_app',target/'deps',extra=extra,env_extra=final_env)
    if family=='rustix' and case.startswith('tamper_'):
        receipts=[p for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('context',{}).get('kind')=='RustixMetadataProbe']
        receipts.sort(key=lambda p:json.loads(p.read_text())['stdin']['template_index'])
        p=receipts[1];r=json.loads(p.read_text());part=case[7:]
        if part in ('stdin','pre','post','missing_snapshot'):
            leaf='stdin.raw' if part=='stdin' else ('metadata-pre.raw' if part=='pre' else 'metadata-post.raw')
            if part=='missing_snapshot':(p.parent/leaf).unlink()
            else:(p.parent/leaf).write_bytes(b'TEST_CODE_CHANGED')
        elif part=='eof':r['stdin']['eof']=False;p.write_text(json.dumps(r))
        elif part=='predecessor':r['predecessor_invocation']=None;p.write_text(json.dumps(r))
        elif part=='declaration':r['declared_outputs'][0]['path']=str(target/'other');p.write_text(json.dumps(r))
    emit({'reason':'build-finished','success':True});sys.exit(0)
'''

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(data)
    return path


def snapshot(root, roots):
    files = {}
    for name in roots:
        path = root / name
        for member in ([path] if path.is_file() else path.rglob("*")):
            if member.is_file():
                files[member.relative_to(root).as_posix()] = sha(member)
    return {"root": str(root), "roots": roots, "files": files}



# Immutable record6 data. Only paths are substituted by this synthetic fixture.
# It does not compile actual indexmap or assert effective deny under cap-lints allow.
INDEXMAP6_PACKAGE = "registry+https://github.com/rust-lang/crates.io-index#indexmap@2.13.1"
INDEXMAP6_LINTS = ["--deny=unsafe-code", "--deny=unreachable-pub", "--deny=unnameable-types",
                  "--allow=clippy::style", "--warn=rust-2018-idioms",
                  "--deny=private-interfaces", "--deny=private-bounds"]
INDEXMAP6_ARGS = ["--crate-name", "indexmap", "--edition=2021", "{source}",
    "--error-format=json", "--json=diagnostic-rendered-ansi,artifacts,future-incompat",
    "--crate-type", "lib", "--emit=dep-info,metadata,link", "-C", "embed-bitcode=no",
    "-C", "debuginfo=1", "-C", "split-debuginfo=unpacked", *INDEXMAP6_LINTS,
    "--cfg", 'feature="default"', "--cfg", 'feature="serde"', "--cfg", 'feature="std"',
    "--check-cfg", 'cfg(docsrs,test)', "--check-cfg",
    'cfg(feature, values("arbitrary", "borsh", "default", "quickcheck", "rayon", "serde", "std", "sval", "test_debug"))',
    "-C", "metadata=d0299001596b1a09", "-C", "extra-filename=-b765f9793bf2700c",
    "--out-dir", "{dest}", "--target", "x86_64-apple-darwin", "-L", "dependency={dest}",
    "-L", "dependency={host}", "--extern", "equivalent={dest}/libequivalent-3595038e0683f798.rmeta",
    "--extern", "hashbrown={dest}/libhashbrown-b209ce0ec345b471.rmeta",
    "--extern", "serde_core={dest}/libserde_core-d3ba454884ccf462.rmeta", "--cap-lints", "allow"]
INDEXMAP6_MANIFEST = ('[package]\nname="indexmap"\nversion="2.13.1"\n'
    '[lints.rust]\nprivate-bounds="deny"\nprivate-interfaces="deny"\n'
    'rust-2018-idioms="warn"\nunnameable-types="deny"\nunreachable-pub="deny"\nunsafe-code="deny"\n'
    '[lints.clippy]\nstyle="allow"\n')
INDEXMAP6_REJECTIONS = {
    "unknown_hyphen": (["--deny=unknown-rust-lint"], "UnsupportedLintArgument"),
    "unsafe_level": (["--allow=unsafe-code"], "UnsupportedLintArgument"),
    "unreachable_level": (["--warn=unreachable-pub"], "UnsupportedLintArgument"),
    "unnameable_level": (["--allow=unnameable-types"], "UnsupportedLintArgument"),
    "interfaces_level": (["--warn=private-interfaces"], "UnsupportedLintArgument"),
    "bounds_level": (["--allow=private-bounds"], "UnsupportedLintArgument"),
    "idioms_level": (["--deny=rust-2018-idioms"], "UnsupportedLintArgument"),
    "double_hyphen": (["--deny=unsafe--code"], "UnsupportedLintArgument"),
    "trailing_hyphen": (["--deny=unsafe-code-"], "UnsupportedLintArgument"),
    "foreign_namespace": (["--deny=other::unsafe-code"], "UnsupportedLintArgument"),
    "clippy_namespace": (["--deny=clippy::unsafe-code"], "UnsupportedLintArgument"),
    "nested_namespace": (["--deny=clippy::rust::unsafe-code"], "UnsupportedLintArgument"),
    "comma": (["--deny=unsafe-code,private-bounds"], "UnsupportedLintArgument"),
    "equals": (["--deny=private-bounds=1"], "UnsupportedLintArgument"),
    "space": (["--deny=unsafe-code "], "UnsupportedLintArgument"),
    "newline": (["--deny=unsafe-code\n"], "UnsupportedLintArgument"),
    "tab": (["--deny=unsafe-code\t"], "UnsupportedLintArgument"),
    "control": (["--deny=unsafe-code\x07"], "UnsupportedLintArgument"),
    "path": (["--deny=../unsafe-code"], "UnsupportedLintArgument"),
    "forbid": (["--forbid=unsafe-code"], "UnsupportedRustcArgument:--forbid=unsafe-code"),
    "force_warn": (["--force-warn=rust-2018-idioms"], "UnsupportedRustcArgument:--force-warn=rust-2018-idioms"),
    "separated": (["--deny", "unsafe-code"], "UnsupportedRustcArgument:--deny"),
    "unknown_codegen": (["-C", "TEST_CODE_unknown=yes"], "UnsupportedCodegen"),
    "unstable": (["-Zrandomize-layout"], "UnsupportedRustcArgument:-Zrandomize-layout"),
}

INDEXMAP6_RUSTC = r"""
import json,os,pathlib,sys
args=sys.argv[1:]
def value(key):
    inline=[a.split('=',1)[1] for a in args if a.startswith(key+'=')]
    return inline[0] if inline else args[args.index(key)+1]
name=value('--crate-name');source=next(pathlib.Path(a) for a in args if a.endswith('.rs'))
codegen=[args[i+1] for i,a in enumerate(args) if a=='-C']
suffix=next((v.split('=',1)[1] for v in codegen if v.startswith('extra-filename=')),'')
base=name+suffix;out=pathlib.Path(value('--out-dir'));out.mkdir(parents=True,exist_ok=True)
hits=pathlib.Path(os.environ['FIXTURE_HIT_ROOT']);hits.mkdir(parents=True,exist_ok=True)
(hits/('compile-'+name)).write_text(json.dumps(args))
for ext in ('.rmeta','.rlib'):(out/('lib'+base+ext)).write_bytes(name.encode()+b':'+source.read_bytes())
def escape(text):return text.replace(chr(92),chr(92)*2).replace(' ',chr(92)+' ').replace('#',chr(92)+'#').replace(':',chr(92)+':').replace('$','$$')
(out/(base+'.d')).write_text(escape(str(out/('lib'+base+'.rlib')))+': '+escape(str(source))+chr(10))
print(json.dumps({'fixture_argv':args}),file=sys.stderr)
"""

INDEXMAP6_CARGO = r"""
import json,os,pathlib,subprocess,sys
CASE=__CASE__;MUTATIONS=__MUTATIONS__;TEMPLATE=__TEMPLATE__;PACKAGE=__PACKAGE__
args=sys.argv[1:];assert args[:6]==['build','--locked','--offline','--lib','--target','x86_64-apple-darwin']
app=pathlib.Path(args[args.index('--manifest-path')+1]).parent;session=app.parent
root=session/'vendor/indexmap';dest=session/'target/x86_64-apple-darwin/debug/deps';host=session/'target/debug/deps'
source=root/'src/lib.rs';original=source.read_bytes()
def emit(value):print(json.dumps(value),flush=True)
def compile(argv,manifest,package):
    env=dict(os.environ,CARGO_MANIFEST_DIR=str(manifest),FIXTURE_HIT_ROOT=str(session/'compiler-entry'),
             DYLD_FALLBACK_LIBRARY_PATH=str(host)+':'+os.environ['DYLD_FALLBACK_LIBRARY_PATH'])
    if manifest==root:env.update(CARGO_PKG_NAME='indexmap',CARGO_PKG_VERSION='2.13.1')
    command=[os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],*argv]
    if manifest==root:
        (session/'fix6-attempt.json').write_text(json.dumps({'argv_hex':[os.fsencode(v).hex() for v in command[1:]]}))
        if CASE=='package_mismatch':env['CARGO_MANIFEST_DIR']=str(app)
        if CASE=='source_mismatch':source.chmod(0o644);source.write_bytes(b'// TEST_CODE_CHANGED\n')
    try:result=subprocess.run(command,env=env,cwd=manifest)
    finally:
        if manifest==root and CASE=='source_mismatch':source.write_bytes(original);source.chmod(0o444)
    if result.returncode:emit({'reason':'build-finished','success':False});sys.exit(result.returncode)
    name=argv[argv.index('--crate-name')+1];src=next(a for a in argv if a.endswith('.rs'))
    suffix=next((v.split('=',1)[1] for v in argv if v.startswith('extra-filename=')),'')
    files=[str(dest/('lib'+name+suffix+ext)) for ext in ('.rlib','.rmeta')]
    emit({'reason':'compiler-artifact','package_id':package,'target':{'src_path':src,'kind':['lib'],'name':name,'crate_types':['lib']},'filenames':files,'executable':None,'fresh':False})
if CASE=='normal':
    for name,suffix in [('equivalent','-3595038e0683f798'),('hashbrown','-b209ce0ec345b471'),('serde_core','-d3ba454884ccf462')]:
        pkg=session/'vendor'/name
        compile(['--crate-name',name,'--crate-type','lib','--emit=dep-info,metadata,link','--out-dir',str(dest),'--target','x86_64-apple-darwin','-C','extra-filename='+suffix,str(pkg/'src/lib.rs')],pkg,'TEST_CODE_'+name)
argv=[v.format(source=source,dest=dest,host=host) for v in TEMPLATE]
if CASE in MUTATIONS:argv=MUTATIONS[CASE][0]+argv
compile(argv,root,PACKAGE)
assert CASE=='normal'
compile(['--crate-name','stock_analysis','--crate-type','lib','--emit=dep-info,metadata,link','--out-dir',str(dest),'--target','x86_64-apple-darwin','--extern','indexmap='+str(dest/'libindexmap-b765f9793bf2700c.rmeta'),str(app/'src/lib.rs')],app,'TEST_CODE_app')
emit({'reason':'build-finished','success':True})
"""

# Record7 argv literals are retained observations; substituted paths, package
# source bytes, sysroot candidates, compiler products and events below are fake.
PROC_MACRO7_ARGS = {'serde_derive': ['--crate-name', 'serde_derive', '--edition=2021', '{session}/vendor/serde_derive/src/lib.rs', '--error-format=json', '--json=diagnostic-rendered-ansi,artifacts,future-incompat', '--crate-type', 'proc-macro', '--emit=dep-info,link', '-C', 'prefer-dynamic', '-C', 'embed-bitcode=no', '-C', 'debuginfo=1', '-C', 'split-debuginfo=unpacked', '--cfg', 'feature="default"', '--check-cfg', 'cfg(docsrs,test)', '--check-cfg', 'cfg(feature, values("default", "deserialize_in_place"))', '-C', 'metadata=736489fbc97c464d', '-C', 'extra-filename=-32debcfbaa2a46e3', '--out-dir', '{session}/target/debug/deps', '-L', 'dependency={session}/target/debug/deps', '--extern', 'proc_macro2={session}/target/debug/deps/libproc_macro2-2e38878fe48e194f.rlib', '--extern', 'quote={session}/target/debug/deps/libquote-75266c67e160dbe3.rlib', '--extern', 'syn={session}/target/debug/deps/libsyn-ad068d60231c3384.rlib', '--extern', 'proc_macro', '--cap-lints', 'allow'], 'tokio_macros': ['--crate-name', 'tokio_macros', '--edition=2021', '{session}/vendor/tokio-macros/src/lib.rs', '--error-format=json', '--json=diagnostic-rendered-ansi,artifacts,future-incompat', '--crate-type', 'proc-macro', '--emit=dep-info,link', '-C', 'prefer-dynamic', '-C', 'embed-bitcode=no', '-C', 'debuginfo=1', '-C', 'split-debuginfo=unpacked', '--warn=unexpected_cfgs', '--check-cfg', 'cfg(fuzzing)', '--check-cfg', 'cfg(loom)', '--check-cfg', 'cfg(mio_unsupported_force_poll_poll)', '--check-cfg', 'cfg(tokio_allow_from_blocking_fd)', '--check-cfg', 'cfg(tokio_internal_mt_counters)', '--check-cfg', 'cfg(tokio_no_parking_lot)', '--check-cfg', 'cfg(tokio_no_tuning_tests)', '--check-cfg', 'cfg(tokio_unstable)', '--check-cfg', 'cfg(target_os, values("cygwin"))', '--check-cfg', 'cfg(docsrs,test)', '--check-cfg', 'cfg(feature, values())', '-C', 'metadata=390ed39a8149c6db', '-C', 'extra-filename=-f1e8bd05b14268be', '--out-dir', '{session}/target/debug/deps', '-L', 'dependency={session}/target/debug/deps', '--extern', 'proc_macro2={session}/target/debug/deps/libproc_macro2-2e38878fe48e194f.rlib', '--extern', 'quote={session}/target/debug/deps/libquote-75266c67e160dbe3.rlib', '--extern', 'syn={session}/target/debug/deps/libsyn-ad068d60231c3384.rlib', '--extern', 'proc_macro', '--cap-lints', 'allow']}
PROC_MACRO7_CANDIDATES = (
    "lib/rustlib/x86_64-apple-darwin/lib/libproc_macro-b94f7a67a9654a0b.rlib",
    "lib/rustlib/x86_64-apple-darwin/lib/libproc_macro-b94f7a67a9654a0b.rmeta")
PROC_MACRO7_RUSTC = "\nimport json,os,pathlib,sys\nargs=sys.argv[1:]\ndef value(key):\n    inline=[a.split('=',1)[1] for a in args if a.startswith(key+'=')]\n    return inline[0] if inline else args[args.index(key)+1]\nname=value('--crate-name');source=next(pathlib.Path(a).resolve() for a in args if a.endswith('.rs'))\ncodegen=[args[i+1] for i,a in enumerate(args) if a=='-C']\nsuffix=next((v.split('=',1)[1] for v in codegen if v.startswith('extra-filename=')),'')\nbase=name+suffix;out=pathlib.Path(value('--out-dir'));out.mkdir(parents=True,exist_ok=True)\nhits=pathlib.Path(os.environ['FIXTURE_HIT_ROOT']);hits.mkdir(parents=True,exist_ok=True)\n(hits/('compile-'+name)).write_text(json.dumps(args))\nfor ext in (('.dylib',) if value('--crate-type')=='proc-macro' else ('.rmeta','.rlib')):(out/('lib'+base+ext)).write_bytes(name.encode()+b':'+source.read_bytes())\ndef escape(text):return text.replace(chr(92),chr(92)*2).replace(' ',chr(92)+' ').replace('#',chr(92)+'#').replace(':',chr(92)+':').replace('$','$$')\n(out/(base+'.d')).write_text(escape(str(out/('lib'+base+'.rlib')))+': '+escape(str(source))+chr(10))\nprint(json.dumps({'fixture_argv':args}),file=sys.stderr)\n"
PROC_MACRO7_REJECTIONS = {
    "unknown": "UnsupportedBareExtern", "core": "UnsupportedBareExtern", "std": "UnsupportedBareExtern",
    "namespace": "UnsupportedBareExtern", "modifier": "UnsupportedBareExtern",
    "space": "UnsupportedBareExtern", "control": "UnsupportedBareExtern",
    "joined": "UnsupportedBareExtern", "duplicate": "BareProcMacroAmbiguous",
    "mixed": "BareProcMacroAmbiguous", "crate_type": "BareProcMacroRole",
    "target": "BareProcMacroRole", "probe": "BareProcMacroRole",
    "out_dir": "BareProcMacroRole", "search": "BareProcMacroRole",
    "emit": "BareProcMacroRole", "nested": "NestedCompilerContext",
    "package": "UnresolvedSourcePackage", "source": "SourceMismatch",
    "manifest": "BareProcMacroOrigin", "version": "BareProcMacroOrigin",
    "manifest_path": "BareProcMacroOrigin", "origin": "BareProcMacroOrigin",
    "candidate_missing": "BareProcMacroSysroot", "candidate_drift": "BareProcMacroSysroot",
    "candidate_alias": "BareProcMacroSysroot", "candidate_extra": "BareProcMacroSysroot",
    "candidate_foreign": "BareProcMacroSysroot"}

PROC_MACRO7_CARGO = r'''
import json,os,pathlib,subprocess,sys
CASE=__CASE__;TEMPLATES=__TEMPLATES__;CANDIDATES=__CANDIDATES__
args=sys.argv[1:];assert args[:6]==['build','--locked','--offline','--lib','--target','x86_64-apple-darwin']
app=pathlib.Path(args[args.index('--manifest-path')+1]).parent;session=app.parent
host=session/'target/debug/deps';dest=session/'target/x86_64-apple-darwin/debug/deps'
sysroot=pathlib.Path(os.environ['DYLD_FALLBACK_LIBRARY_PATH']).parent
events=[]
def event(value): events.append(value)
def finish(success):
    for value in events:print(json.dumps(value),flush=True)
    print(json.dumps({'reason':'build-finished','success':success}),flush=True)
def compile(argv,root,package,macro=False):
    env=dict(os.environ,CARGO_MANIFEST_DIR=str(root),CARGO_MANIFEST_PATH=str(root/'Cargo.toml'),
             FIXTURE_HIT_ROOT=str(session/'compiler-entry'),
             DYLD_FALLBACK_LIBRARY_PATH=str(host)+':'+os.environ['DYLD_FALLBACK_LIBRARY_PATH'])
    name=argv[argv.index('--crate-name')+1];restores=[]
    if macro:
        env.update(CARGO_PKG_NAME=root.name,CARGO_PKG_VERSION='1.0.228' if root.name=='serde_derive' else '2.7.0')
        if CASE=='package':env['CARGO_MANIFEST_DIR']=str(app)
        if CASE=='version':env['CARGO_PKG_VERSION']='0.0.0'
        if CASE=='manifest_path':env['CARGO_MANIFEST_PATH']=str(app/'Cargo.toml')
        if CASE=='nested':env['DYLD_FALLBACK_LIBRARY_PATH']=str(session/'target/debug')+':'+env['DYLD_FALLBACK_LIBRARY_PATH']
        changed=None
        if CASE in ('source','manifest'):changed=root/('src/lib.rs' if CASE=='source' else 'Cargo.toml')
        if CASE in ('candidate_drift','candidate_alias'):changed=sysroot/CANDIDATES[0]
        if changed:
            restores.append((changed,changed.read_bytes(),changed.stat().st_mode & 0o777));changed.chmod(0o644)
            if CASE=='candidate_alias':changed.unlink();changed.symlink_to(sysroot/CANDIDATES[1])
            else:changed.write_bytes(b'// TEST_CODE drift\n')
    command=[os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],*argv]
    if macro:(session/('fix7-attempt-'+name+'.json')).write_text(json.dumps({'argv_hex':[os.fsencode(v).hex() for v in command[1:]]}))
    try:result=subprocess.run(command,cwd=root,env=env)
    finally:
        for path,body,mode in restores:
            if path.is_symlink():path.unlink()
            path.write_bytes(body);path.chmod(mode)
    if result.returncode:finish(False);sys.exit(result.returncode)
    out=pathlib.Path(argv[argv.index('--out-dir')+1]);source=next((root/a).resolve() for a in argv if a.endswith('.rs'))
    suffix=next((v.split('=',1)[1] for v in argv if v.startswith('extra-filename=')),'')
    kind='proc-macro' if macro else 'lib';exts=['.dylib'] if macro else ['.rlib','.rmeta']
    event({'reason':'compiler-artifact','package_id':package,'target':{'src_path':str(source),'kind':[kind],
          'crate_types':[kind],'name':name},'filenames':[str(out/('lib'+name+suffix+ext)) for ext in exts],
          'executable':None,'fresh':False})
positive=CASE in ('normal','qualified','unresolved') or CASE.startswith('seal_')
if positive:
    for name,suffix in [('proc_macro2','-2e38878fe48e194f'),('quote','-75266c67e160dbe3'),('syn','-ad068d60231c3384')]:
        root=session/'vendor'/name
        compile(['--crate-name',name,'--crate-type','lib','--emit=dep-info,metadata,link','--out-dir',str(host),
                 '-C','extra-filename='+suffix,str(root/'src/lib.rs')],root,'TEST_CODE_'+name)
for crate,name,version in [('serde_derive','serde_derive','1.0.228'),('tokio_macros','tokio-macros','2.7.0')]:
    root=session/'vendor'/name
    argv=[v.replace('{session}',str(session)).replace('{sysroot}',str(sysroot)) for v in TEMPLATES[crate]]
    at=argv.index('proc_macro');flag=at-1
    replacements={'unknown':'proc_macro2','core':'core','std':'std','namespace':'other::proc_macro','modifier':'priv:proc_macro',
                  'space':'proc_macro ','control':'proc_macro\t'}
    if CASE in replacements:argv[at]=replacements[CASE]
    if CASE=='joined':argv[flag:at+1]=['--extern=proc_macro']
    if CASE=='duplicate':argv[flag:flag]=['--extern','proc_macro']
    if CASE=='mixed':argv[flag:flag]=['--extern','proc_macro='+str(host/'libproc_macro.rlib')]
    if CASE=='qualified':argv[at]='proc_macro='+str(host/'libproc_macro2-2e38878fe48e194f.rlib')
    if CASE=='crate_type':argv[argv.index('--crate-type')+1]='lib'
    if CASE=='target':argv.extend(['--target','x86_64-apple-darwin'])
    if CASE=='probe':argv=['--version','--extern','proc_macro']
    if CASE=='out_dir':argv[argv.index('--out-dir')+1]=str(dest)
    if CASE=='search':argv[argv.index('-L')+1]='dependency='+str(dest)
    if CASE=='emit':argv[argv.index('--emit=dep-info,link')]='--emit=dep-info,metadata,link'
    if CASE=='unresolved':argv[argv.index('proc_macro2='+str(host/'libproc_macro2-2e38878fe48e194f.rlib'))]='proc_macro2='+str(host/'unproduced.rlib')
    if CASE=='probe':
        # compile's only --crate-name read is bookkeeping, so this raw probe is
        # sent separately and must still stop before the fake compiler entry.
        env=dict(os.environ,CARGO_MANIFEST_DIR=str(root),DYLD_FALLBACK_LIBRARY_PATH=os.environ['DYLD_FALLBACK_LIBRARY_PATH'])
        command=[os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],*argv]
        (session/('fix7-attempt-'+crate+'.json')).write_text(json.dumps({'argv_hex':[os.fsencode(v).hex() for v in command[1:]]}))
        result=subprocess.run(command,cwd=root,env=env);finish(False);sys.exit(result.returncode)
    compile(argv,root,'registry+https://github.com/rust-lang/crates.io-index#'+name+'@'+version,True)
    if not positive:raise AssertionError('negative case reached compiler')
compile(['--crate-name','stock_analysis','--crate-type','lib','--emit=dep-info,metadata,link','--out-dir',str(dest),
         '--target','x86_64-apple-darwin',str(app/'src/lib.rs')],app,'TEST_CODE_app')
if CASE.startswith('seal_'):
    call=next(p.parent for p in (session/'invocations').glob('*/receipt.json')
              if json.loads(p.read_text())['parsed']['options'].get('--crate-name')==['serde_derive'])
    (session/'fix7-seal-call').write_text(call.name)
    event_index=next(i for i,e in enumerate(events) if e['target']['name']=='serde_derive')
    if CASE=='seal_missing_event':events.pop(event_index)
    elif CASE=='seal_duplicate_event':events.append(events[event_index])
    elif CASE in ('seal_wrong_kind','seal_wrong_crate_type','seal_wrong_name','seal_wrong_source','seal_wrong_package'):
        e=events[event_index]
        if CASE=='seal_wrong_kind':e['target']['kind']=['lib']
        if CASE=='seal_wrong_crate_type':e['target']['crate_types']=['lib']
        if CASE=='seal_wrong_name':e['target']['name']='other'
        if CASE=='seal_wrong_source':e['target']['src_path']=str(app/'src/lib.rs')
        if CASE=='seal_wrong_package':e['package_id']='TEST_CODE_other'
    else:
        filename='request.json' if CASE in ('seal_raw','seal_environment') else ('invocation.json' if CASE=='seal_initial' else 'receipt.json')
        path=call/filename;data=json.loads(path.read_text())
        if CASE in ('seal_missing','seal_initial','seal_all_missing'):
            data.pop('sysroot_extern_declarations')
            if CASE=='seal_all_missing':
                initial=call/'invocation.json';item=json.loads(initial.read_text());item.pop('sysroot_extern_declarations');initial.write_text(json.dumps(item))
        elif CASE=='seal_nonzero':data['exit_code']=7
        elif CASE=='seal_role':data['role']='Target'
        elif CASE=='seal_source':data['source']=str(app/'src/lib.rs')
        elif CASE=='seal_package':data['package']['id']='TEST_CODE_other'
        elif CASE=='seal_raw':data['argv_hex'][-1]=b'warn'.hex()
        elif CASE=='seal_environment':data['environment_hex'][b'CARGO_PKG_VERSION'.hex()]=b'0'.hex()
        elif CASE=='seal_candidate':data['sysroot_extern_declarations'][0]['candidates'][0]['sha256']='0'*64
        elif CASE=='seal_selected':data['sysroot_extern_declarations'][0]['artifact_selection']='opened'
        path.write_text(json.dumps(data))
finish(True)
'''


# Frozen metadata rows, copied as independent test expectations; fixtures below
# are synthetic and do not claim these packages compiled in a real recording.
PROC_MACRO8_ROWS = (
    ('registry+https://github.com/rust-lang/crates.io-index#async-stream-impl@0.3.6', 'async_stream_impl', 'async-stream-impl', '0.3.6', 'async-stream-impl/Cargo.toml', 'async-stream-impl/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#async-trait@0.1.89', 'async_trait', 'async-trait', '0.1.89', 'async-trait/Cargo.toml', 'async-trait/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#bincode_derive@2.0.1', 'bincode_derive', 'bincode_derive', '2.0.1', 'bincode_derive/Cargo.toml', 'bincode_derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#bytemuck_derive@1.10.2', 'bytemuck_derive', 'bytemuck_derive', '1.10.2', 'bytemuck_derive/Cargo.toml', 'bytemuck_derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#clap_derive@4.6.0', 'clap_derive', 'clap_derive', '4.6.0', 'clap_derive/Cargo.toml', 'clap_derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#darling_macro@0.14.4', 'darling_macro', 'darling_macro', '0.14.4', 'darling_macro-0.14.4/Cargo.toml', 'darling_macro-0.14.4/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#darling_macro@0.21.3', 'darling_macro', 'darling_macro', '0.21.3', 'darling_macro/Cargo.toml', 'darling_macro/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#derive_builder_macro@0.12.0', 'derive_builder_macro', 'derive_builder_macro', '0.12.0', 'derive_builder_macro/Cargo.toml', 'derive_builder_macro/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#diesel_derives@2.3.7', 'diesel_derives', 'diesel_derives', '2.3.7', 'diesel_derives/Cargo.toml', 'diesel_derives/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#displaydoc@0.2.5', 'displaydoc', 'displaydoc', '0.2.5', 'displaydoc/Cargo.toml', 'displaydoc/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#document-features@0.2.12', 'document_features', 'document-features', '0.2.12', 'document-features/Cargo.toml', 'document-features/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#foreign-types-macros@0.2.3', 'foreign_types_macros', 'foreign-types-macros', '0.2.3', 'foreign-types-macros/Cargo.toml', 'foreign-types-macros/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#futures-macro@0.3.32', 'futures_macro', 'futures-macro', '0.3.32', 'futures-macro/Cargo.toml', 'futures-macro/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#jiff-static@0.2.23', 'jiff_static', 'jiff-static', '0.2.23', 'jiff-static/Cargo.toml', 'jiff-static/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#num-derive@0.4.2', 'num_derive', 'num-derive', '0.4.2', 'num-derive/Cargo.toml', 'num-derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#openssl-macros@0.1.1', 'openssl_macros', 'openssl-macros', '0.1.1', 'openssl-macros/Cargo.toml', 'openssl-macros/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#pin-project-internal@1.1.13', 'pin_project_internal', 'pin-project-internal', '1.1.13', 'pin-project-internal/Cargo.toml', 'pin-project-internal/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#prost-derive@0.14.4', 'prost_derive', 'prost-derive', '0.14.4', 'prost-derive/Cargo.toml', 'prost-derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#recursive-proc-macro-impl@0.1.1', 'recursive_proc_macro_impl', 'recursive-proc-macro-impl', '0.1.1', 'recursive-proc-macro-impl/Cargo.toml', 'recursive-proc-macro-impl/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#rustversion@1.0.22', 'rustversion', 'rustversion', '1.0.22', 'rustversion/Cargo.toml', 'rustversion/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#serde_derive@1.0.228', 'serde_derive', 'serde_derive', '1.0.228', 'serde_derive/Cargo.toml', 'serde_derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#serial_test_derive@0.10.0', 'serial_test_derive', 'serial_test_derive', '0.10.0', 'serial_test_derive/Cargo.toml', 'serial_test_derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#sqlparser_derive@0.4.0', 'sqlparser_derive', 'sqlparser_derive', '0.4.0', 'sqlparser_derive/Cargo.toml', 'sqlparser_derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#strum_macros@0.27.2', 'strum_macros', 'strum_macros', '0.27.2', 'strum_macros/Cargo.toml', 'strum_macros/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#thiserror-impl@1.0.69', 'thiserror_impl', 'thiserror-impl', '1.0.69', 'thiserror-impl-1.0.69/Cargo.toml', 'thiserror-impl-1.0.69/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#thiserror-impl@2.0.18', 'thiserror_impl', 'thiserror-impl', '2.0.18', 'thiserror-impl/Cargo.toml', 'thiserror-impl/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#time-macros@0.2.32', 'time_macros', 'time-macros', '0.2.32', 'time-macros/Cargo.toml', 'time-macros/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#tokio-macros@2.7.0', 'tokio_macros', 'tokio-macros', '2.7.0', 'tokio-macros/Cargo.toml', 'tokio-macros/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#tracing-attributes@0.1.31', 'tracing_attributes', 'tracing-attributes', '0.1.31', 'tracing-attributes/Cargo.toml', 'tracing-attributes/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#wasm-bindgen-macro@0.2.117', 'wasm_bindgen_macro', 'wasm-bindgen-macro', '0.2.117', 'wasm-bindgen-macro/Cargo.toml', 'wasm-bindgen-macro/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#windows-implement@0.60.2', 'windows_implement', 'windows-implement', '0.60.2', 'windows-implement/Cargo.toml', 'windows-implement/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#windows-interface@0.59.3', 'windows_interface', 'windows-interface', '0.59.3', 'windows-interface/Cargo.toml', 'windows-interface/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#wit-bindgen-rust-macro@0.51.0', 'wit_bindgen_rust_macro', 'wit-bindgen-rust-macro', '0.51.0', 'wit-bindgen-rust-macro/Cargo.toml', 'wit-bindgen-rust-macro/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#yoke-derive@0.8.2', 'yoke_derive', 'yoke-derive', '0.8.2', 'yoke-derive/Cargo.toml', 'yoke-derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#zerocopy-derive@0.8.48', 'zerocopy_derive', 'zerocopy-derive', '0.8.48', 'zerocopy-derive/Cargo.toml', 'zerocopy-derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#zerofrom-derive@0.1.7', 'zerofrom_derive', 'zerofrom-derive', '0.1.7', 'zerofrom-derive/Cargo.toml', 'zerofrom-derive/src/lib.rs'),
    ('registry+https://github.com/rust-lang/crates.io-index#zerovec-derive@0.11.3', 'zerovec_derive', 'zerovec-derive', '0.11.3', 'zerovec-derive/Cargo.toml', 'zerovec-derive/src/lib.rs'),
)

# Exact record8 argv after compiler token, with only the session path replaced.
PROC_MACRO8_OBSERVED_ARGS = {'futures_macro': ['--crate-name', 'futures_macro', '--edition=2018', '{session}/vendor/futures-macro/src/lib.rs', '--error-format=json', '--json=diagnostic-rendered-ansi,artifacts,future-incompat', '--crate-type', 'proc-macro', '--emit=dep-info,link', '-C', 'prefer-dynamic', '-C', 'embed-bitcode=no', '-C', 'debuginfo=1', '-C', 'split-debuginfo=unpacked', '--warn=unreachable_pub', '--warn=unexpected_cfgs', '--warn=single_use_lifetimes', '--warn=rust_2018_idioms', '--warn=missing_debug_implementations', '--check-cfg', 'cfg(futures_sanitizer)', '--check-cfg', 'cfg(docsrs,test)', '--check-cfg', 'cfg(feature, values())', '-C', 'metadata=59f95555403d985f', '-C', 'extra-filename=-8f4838224de05cd8', '--out-dir', '{session}/target/debug/deps', '-L', 'dependency={session}/target/debug/deps', '--extern', 'proc_macro2={session}/target/debug/deps/libproc_macro2-2e38878fe48e194f.rlib', '--extern', 'quote={session}/target/debug/deps/libquote-75266c67e160dbe3.rlib', '--extern', 'syn={session}/target/debug/deps/libsyn-ad068d60231c3384.rlib', '--extern', 'proc_macro', '--cap-lints', 'allow'], 'tracing_attributes': ['--crate-name', 'tracing_attributes', '--edition=2018', '{session}/vendor/tracing-attributes/src/lib.rs', '--error-format=json', '--json=diagnostic-rendered-ansi,artifacts,future-incompat', '--crate-type', 'proc-macro', '--emit=dep-info,link', '-C', 'prefer-dynamic', '-C', 'embed-bitcode=no', '-C', 'debuginfo=1', '-C', 'split-debuginfo=unpacked', '--warn=unexpected_cfgs', '--check-cfg', 'cfg(flaky_tests)', '--check-cfg', 'cfg(tracing_unstable)', '--check-cfg', 'cfg(unsound_local_offset)', '--check-cfg', 'cfg(docsrs,test)', '--check-cfg', 'cfg(feature, values("async-await"))', '-C', 'metadata=7ef6e947376c2f71', '-C', 'extra-filename=-6cab56d7811dfd7c', '--out-dir', '{session}/target/debug/deps', '-L', 'dependency={session}/target/debug/deps', '--extern', 'proc_macro2={session}/target/debug/deps/libproc_macro2-2e38878fe48e194f.rlib', '--extern', 'quote={session}/target/debug/deps/libquote-75266c67e160dbe3.rlib', '--extern', 'syn={session}/target/debug/deps/libsyn-ad068d60231c3384.rlib', '--extern', 'proc_macro', '--cap-lints', 'allow'], 'displaydoc': ['--crate-name', 'displaydoc', '--edition=2021', '{session}/vendor/displaydoc/src/lib.rs', '--error-format=json', '--json=diagnostic-rendered-ansi,artifacts,future-incompat', '--crate-type', 'proc-macro', '--emit=dep-info,link', '-C', 'prefer-dynamic', '-C', 'embed-bitcode=no', '-C', 'debuginfo=1', '-C', 'split-debuginfo=unpacked', '--check-cfg', 'cfg(docsrs,test)', '--check-cfg', 'cfg(feature, values("default", "std"))', '-C', 'metadata=3da73696a8601065', '-C', 'extra-filename=-811213a680831292', '--out-dir', '{session}/target/debug/deps', '-L', 'dependency={session}/target/debug/deps', '--extern', 'proc_macro2={session}/target/debug/deps/libproc_macro2-2e38878fe48e194f.rlib', '--extern', 'quote={session}/target/debug/deps/libquote-75266c67e160dbe3.rlib', '--extern', 'syn={session}/target/debug/deps/libsyn-ad068d60231c3384.rlib', '--extern', 'proc_macro', '--cap-lints', 'allow'], 'zerovec_derive': ['--crate-name', 'zerovec_derive', '--edition=2021', '{session}/vendor/zerovec-derive/src/lib.rs', '--error-format=json', '--json=diagnostic-rendered-ansi,artifacts,future-incompat', '--crate-type', 'proc-macro', '--emit=dep-info,link', '-C', 'prefer-dynamic', '-C', 'embed-bitcode=no', '-C', 'debuginfo=1', '-C', 'split-debuginfo=unpacked', '--warn=clippy::wildcard_dependencies', '--warn=clippy::useless_transmute', '--warn=unused_qualifications', '--warn=unused_macro_rules', '--warn=unused_lifetimes', '--warn=clippy::unnecessary-wraps', '--warn=unexpected_cfgs', '--deny=clippy::trivially_copy_pass_by_ref', '--deny=trivial_numeric_casts', '--warn=clippy::transmutes_expressible_as_ptr_casts', '--warn=clippy::transmute_undefined_repr', '--warn=clippy::transmute_ptr_to_ref', '--warn=clippy::transmute_ptr_to_ptr', '--warn=clippy::transmute_int_to_non_zero', '--warn=clippy::transmute_int_to_bool', '--warn=clippy::transmute_bytes_to_str', '--warn=clippy::todo', '--warn=clippy::same_functions_in_if_condition', '--warn=clippy::or-fun-call', '--warn=clippy::negative_feature_names', '--warn=clippy::missing_transmute_annotations', '--warn=clippy::missing_fields_in_debug', '--deny=missing_debug_implementations', '--warn=clippy::mismatching_type_param_order', '--warn=clippy::large_stack_arrays', '--warn=clippy::infinite_loop', '--warn=clippy::fn_to_numeric_cast_any', '--deny=clippy::exhaustive_structs', '--deny=clippy::exhaustive_enums', '--warn=clippy::doc_markdown', '--warn=clippy::debug_assert_with_mut_call', '--warn=clippy::dbg_macro', '--warn=clippy::crosspointer_transmute', '--warn=clippy::collection_is_never_read', '--warn=clippy::branches-sharing-code', '--warn=clippy::alloc-instead-of-core', '--check-cfg', 'cfg(icu4c_enable_renaming)', '--check-cfg', 'cfg(needs_alloc_error_handler)', '--check-cfg', 'cfg(icu4x_run_size_tests)', '--check-cfg', 'cfg(icu4x_unstable_fast_trie_only)', '--check-cfg', 'cfg(docsrs,test)', '--check-cfg', 'cfg(feature, values())', '-C', 'metadata=699e703cb9df9315', '-C', 'extra-filename=-6deafd149e977212', '--out-dir', '{session}/target/debug/deps', '-L', 'dependency={session}/target/debug/deps', '--extern', 'proc_macro2={session}/target/debug/deps/libproc_macro2-2e38878fe48e194f.rlib', '--extern', 'quote={session}/target/debug/deps/libquote-75266c67e160dbe3.rlib', '--extern', 'syn={session}/target/debug/deps/libsyn-ad068d60231c3384.rlib', '--extern', 'proc_macro', '--cap-lints', 'allow']}
PROC_MACRO8_REQUEST_SHA256 = {'futures_macro': '0f72a0af88fca6629d7f893bf8472d48728cad35fd805d709a5897320858db8c', 'tracing_attributes': 'e42bb368c37efd01ac80ade3b71bc9f5a8e5ffcec6c68d720f108e5660e29a5e', 'displaydoc': 'e47a49ac21d5f39962198321d6ae6ce33b096902703b380e6ab7f28efb66deae', 'zerovec_derive': '69993f8eb14041119a75b053343886b161801298b8d4d7b73efc05fa0dba6c66'}


# Separate fake executables; no change to the earlier recording fixture helpers.
# Versioned macros get distinct output/hit names, so same-name calls cannot mask each other.
PROC_MACRO8_RUSTC = PROC_MACRO7_RUSTC.replace("('compile-'+name)", "('compile-'+base)")
PROC_MACRO8_CARGO = r'''
import json,os,pathlib,subprocess,sys
CASE=__CASE__;ROWS=__ROWS__;TEMPLATES=__TEMPLATES__
args=sys.argv[1:];assert args[:6]==['build','--locked','--offline','--lib','--target','x86_64-apple-darwin']
app=pathlib.Path(args[args.index('--manifest-path')+1]).parent;session=app.parent
host=session/'target/debug/deps';dest=session/'target/x86_64-apple-darwin/debug/deps'
events=[];macro_calls=[]
def finish(ok):
    for e in events:print(json.dumps(e),flush=True)
    print(json.dumps({'reason':'build-finished','success':ok}),flush=True)
def compile(argv,root,package,manifest,name=None,version=None):
    macro=name is not None
    env=dict(os.environ,CARGO_MANIFEST_DIR=str(root),CARGO_MANIFEST_PATH=str(manifest),
             FIXTURE_HIT_ROOT=str(session/'compiler-entry'),
             DYLD_FALLBACK_LIBRARY_PATH=str(host)+':'+os.environ['DYLD_FALLBACK_LIBRARY_PATH'])
    if macro:env.update(CARGO_PKG_NAME=name,CARGO_PKG_VERSION=version)
    source=next((root/a).resolve() for a in argv if a.endswith('.rs'))
    restores=[]
    if macro:
        if CASE=='reject_env_name':env['CARGO_PKG_NAME']='other'
        if CASE=='reject_env_version':env['CARGO_PKG_VERSION']='0.0.0'
        if CASE=='reject_env_manifest':env['CARGO_MANIFEST_PATH']=str(app/'Cargo.toml')
        if CASE=='reject_source_package':env['CARGO_MANIFEST_DIR']=str(app)
        changed=source if CASE=='reject_source_drift' else (manifest if CASE=='reject_manifest_drift' else None)
        if changed:
            restores.append((changed,changed.read_bytes(),changed.stat().st_mode & 0o777))
            changed.chmod(0o644);changed.write_bytes(b'// TEST_CODE drift\n')
    command=[os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],*argv]
    if macro:
        (session/('fix8-attempt-'+str(len(macro_calls))+'.json')).write_text(json.dumps(
            {'package_id':package,'argv_hex':[os.fsencode(v).hex() for v in command[1:]]}))
    try:result=subprocess.run(command,cwd=root,env=env)
    finally:
        for path,body,mode in restores:path.write_bytes(body);path.chmod(mode)
    if result.returncode:finish(False);sys.exit(result.returncode)
    crate=argv[argv.index('--crate-name')+1];out=pathlib.Path(argv[argv.index('--out-dir')+1])
    suffix=next((v.split('=',1)[1] for v in argv if v.startswith('extra-filename=')),'')
    kind='proc-macro' if macro else 'lib';exts=['.dylib'] if macro else ['.rlib','.rmeta']
    events.append({'reason':'compiler-artifact','package_id':package,'target':{'src_path':str(source),
        'kind':[kind],'crate_types':[kind],'name':crate},
        'filenames':[str(out/('lib'+crate+suffix+ext)) for ext in exts],'executable':None,'fresh':False})
    if macro:
        call=next(f.parent for f in (session/'invocations').glob('*/receipt.json')
                  if json.loads(f.read_text())['package']['id']==package)
        macro_calls.append(call)
        (session/'fix8-macro-calls.json').write_text(json.dumps([str(c.name) for c in macro_calls]))
if not CASE.startswith('reject_'):
    for name,suffix in [('proc_macro2','-2e38878fe48e194f'),('quote','-75266c67e160dbe3'),('syn','-ad068d60231c3384')]:
        root=session/'vendor'/name
        compile(['--crate-name',name,'--crate-type','lib','--emit=dep-info,metadata,link','--out-dir',str(host),
            '-C','extra-filename='+suffix,str(root/'src/lib.rs')],root,'TEST_CODE_'+name,root/'Cargo.toml')
for i,(pid,crate,name,version,manifest_relative,source_relative) in enumerate(ROWS):
    manifest=session/'vendor'/manifest_relative;root=manifest.parent;source=session/'vendor'/source_relative
    if CASE=='observed':
        argv=[v.replace('{session}',str(session)) for v in TEMPLATES[crate]]
    else:
        argv=['--crate-name',crate,'--edition=2021',str(source),'--crate-type','proc-macro',
              '--emit=dep-info,link','-C','extra-filename=-fix8-'+str(i),'--out-dir',str(host),'-L','dependency='+str(host),
              '--extern','proc_macro2='+str(host/'libproc_macro2-2e38878fe48e194f.rlib'),
              '--extern','quote='+str(host/'libquote-75266c67e160dbe3.rlib'),
              '--extern','syn='+str(host/'libsyn-ad068d60231c3384.rlib'),'--extern','proc_macro','--cap-lints','allow']
    compile(argv,root,pid,manifest,name,version)
    if CASE.startswith('reject_'):raise AssertionError('negative reached compiler')
compile(['--crate-name','stock_analysis','--crate-type','lib','--emit=dep-info,metadata,link','--out-dir',str(dest),
         '--target','x86_64-apple-darwin',str(app/'src/lib.rs')],app,'TEST_CODE_app',app/'Cargo.toml')
if CASE.startswith('seal_'):
    call=macro_calls[0];pid=ROWS[0][0];at=next(i for i,e in enumerate(events) if e['package_id']==pid)
    if CASE=='seal_missing_event':events.pop(at)
    elif CASE=='seal_duplicate_event':events.append(events[at])
    elif CASE=='seal_wrong_package':events[at]['package_id']='TEST_CODE_wrong'
    elif CASE=='seal_wrong_source':events[at]['target']['src_path']=str(app/'src/lib.rs')
    elif CASE=='seal_swap_versions':
        other=next(i for i,e in enumerate(events) if e['package_id']==ROWS[1][0])
        events[at]['package_id'],events[other]['package_id']=events[other]['package_id'],events[at]['package_id']
    elif CASE=='seal_missing_extern':
        producer=next(f for f in (session/'invocations').glob('*/receipt.json')
                      if json.loads(f.read_text())['package']['id']=='TEST_CODE_proc_macro2')
        # Missing actual receipt, not just a missing Cargo event: ordinary
        # output ownership is established from successful compiler receipts.
        producer.unlink()
    else:
        filename='request.json' if CASE=='seal_raw' else 'receipt.json'
        path=call/filename;data=json.loads(path.read_text())
        if CASE=='seal_raw':data['argv_hex'][-1]=b'warn'.hex()
        elif CASE=='seal_declaration':data['sysroot_extern_declarations'][0]['artifact_selection']='opened'
        else:raise AssertionError(CASE)
        path.write_text(json.dumps(data))
finish(True)
'''

# Record9 literal argv data; fake archives/compilers below do not claim native provenance.
RING9_ARGS = ['--crate-name', 'ring', '--edition=2021', '{source}', '--error-format=json', '--json=diagnostic-rendered-ansi,artifacts,future-incompat', '--crate-type', 'lib', '--emit=dep-info,metadata,link', '-C', 'embed-bitcode=no', '-C', 'debuginfo=1', '-C', 'split-debuginfo=unpacked', '--cfg', 'feature="alloc"', '--cfg', 'feature="default"', '--cfg', 'feature="dev_urandom_fallback"', '--cfg', 'feature="std"', '--check-cfg', 'cfg(docsrs,test)', '--check-cfg', 'cfg(feature, values("alloc", "default", "dev_urandom_fallback", "less-safe-getrandom-custom-or-rdrand", "less-safe-getrandom-espidf", "slow_tests", "std", "test_logging", "unstable-testing-arm-no-hw", "unstable-testing-arm-no-neon", "wasm32_unknown_unknown_js"))', '-C', 'metadata=862fd12a131d46d1', '-C', 'extra-filename=-caa7295bfcdb1ff4', '--out-dir', '{deps}', '--target', 'x86_64-apple-darwin', '-L', 'dependency={deps}', '-L', 'dependency={host}', '--extern', 'cfg_if={deps}/libcfg_if-cbe7c8951fbba95c.rmeta', '--extern', 'getrandom={deps}/libgetrandom-7d22719848931b99.rmeta', '--extern', 'untrusted={deps}/libuntrusted-3d1aaf426382159e.rmeta', '--cap-lints', 'allow', '-L', 'native={out}', '-l', 'static=ring_core_0_17_14_', '-l', 'static=ring_core_0_17_14__test']

RING9_RUSTC = r'''
import json,os,pathlib,sys
args=sys.argv[1:]
if args==['--print','sysroot']:
    hits=pathlib.Path(os.environ['FIXTURE_HIT_ROOT']);hits.mkdir(parents=True,exist_ok=True)
    (hits/'probe-ring').write_text(json.dumps(args))
    print(pathlib.Path(sys.argv[0]).parent/'sysroot');sys.exit(0)
def value(k):
    inline=[a.split('=',1)[1] for a in args if a.startswith(k+'=')]
    return inline[0] if inline else args[args.index(k)+1]
name=value('--crate-name');source=next(pathlib.Path(a) for a in args if a.endswith('.rs'))
hits=pathlib.Path(os.environ['FIXTURE_HIT_ROOT']);hits.mkdir(parents=True,exist_ok=True)
(hits/('compile-'+name)).write_text(json.dumps(args))
case=os.environ.get('RING9_CASE','normal')
if name=='ring':
    archive=pathlib.Path(os.environ['OUT_DIR'])/'libring_core_0_17_14_.a'
    if case=='during_change':archive.write_bytes(b'TEST_CODE_CHANGED_DURING_CHILD')
    if case=='post_missing':archive.unlink()
out=pathlib.Path(value('--out-dir'));out.mkdir(parents=True,exist_ok=True)
codegen=[args[i+1] for i,a in enumerate(args) if a=='-C']
suffix=next((a.split('=',1)[1] for a in codegen if a.startswith('extra-filename=')),'')
base=name+suffix
files=[out/base] if value('--crate-type')=='bin' else [out/('lib'+base+'.rmeta'),out/('lib'+base+'.rlib')]
for path in files:path.write_bytes(b'TEST_CODE_OUTPUT:'+name.encode()+b':'+source.read_bytes())
def esc(s):return s.replace(chr(92),chr(92)*2).replace(' ',chr(92)+' ').replace('#',chr(92)+'#').replace(':',chr(92)+':').replace('$','$$')
(out/(base+'.d')).write_text(esc(str(files[0]))+': '+esc(str(source))+'\n')
print(json.dumps({'fixture_argv':args}),file=sys.stderr)
if name=='ring' and case=='compiler_fail':sys.exit(7)
'''

RING9_CARGO = r'''
import json,os,pathlib,shutil,subprocess,sys
CASE=__CASE__;TEMPLATE=__TEMPLATE__
args=sys.argv[1:]
def value(k):return args[args.index(k)+1]
app=pathlib.Path(value('--manifest-path')).parent;session=app.parent;target=pathlib.Path(value('--target-dir'))
root=session/'vendor/ring';host=target/'debug/deps';deps=target/'x86_64-apple-darwin/debug/deps'
for p in (host,deps):p.mkdir(parents=True,exist_ok=True)
features=['alloc','default','dev_urandom_fallback','std'];cfg=[v for f in features for v in ('--cfg','feature="'+f+'"')]
package='registry+https://github.com/rust-lang/crates.io-index#ring@0.17.14'
libs=['static=ring_core_0_17_14_','static=ring_core_0_17_14__test']
out=target/'x86_64-apple-darwin/debug/build/ring-a2bdcca0b9c169e7/out';out.mkdir(parents=True)
loader=str(host)+':'+os.environ['DYLD_FALLBACK_LIBRARY_PATH']
def emit(e):print(json.dumps(e),flush=True)
def artifact(pkg,source,name,kind,files,feats):
    return {'reason':'compiler-artifact','package_id':pkg,'manifest_path':str(source.parent/'Cargo.toml'),
        'target':{'src_path':str(source),'kind':[kind],'crate_types':['bin' if kind=='custom-build' else kind],'name':name,'edition':'2021'},
        'features':feats,'filenames':[str(p) for p in files],'executable':None,'fresh':False}
def compile(name,source,pkg,dest,kind='lib',extra=()):
    command=[os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],'--crate-name',name,'--edition=2021',str(source),'--crate-type',kind,'--emit='+('dep-info,link' if kind=='bin' else 'dep-info,metadata,link'),'--out-dir',str(dest),*extra]
    env=dict(os.environ,CARGO_MANIFEST_DIR=str(source.parent if source.name=='build.rs' else source.parent.parent),DYLD_FALLBACK_LIBRARY_PATH=loader,FIXTURE_HIT_ROOT=str(session/'compiler-entry'))
    if pkg==package:
        env.update(CARGO_PKG_NAME='ring',CARGO_PKG_VERSION='0.17.14')
    result=subprocess.run(command,env=env,cwd=pathlib.Path(env['CARGO_MANIFEST_DIR']))
    if result.returncode:raise RuntimeError('TEST_CODE prerequisite compile failed')
    files=[dest/name] if kind=='bin' else [dest/('lib'+name+'.rmeta'),dest/('lib'+name+'.rlib')]
    event=artifact(pkg,source,name,'custom-build' if kind=='bin' else kind,files,features if pkg==package else [])
    if kind=='bin':
        alias=dest/'build-script-build';shutil.copyfile(files[0],alias);event['filenames']=[str(alias)]
    return event
ccpkg='registry+https://github.com/rust-lang/crates.io-index#cc@1.2.59'
emit(compile('cc',session/'vendor/cc/src/lib.rs',ccpkg,host))
builder=compile('build_script_build',root/'build.rs',package,target/'debug/build/ring-89fcb6af8728a6c7','bin',cfg+['--extern','cc='+str(host/'libcc.rlib')])
if CASE=='builder_features':builder['features']=[]
if CASE=='builder_package':builder['package_id']='TEST_CODE_OTHER'
if CASE!='missing_builder':emit(builder)
if CASE=='duplicate_builder':emit(builder)
for name in ('cfg_if','getrandom','untrusted'):
    source=session/'vendor'/name/'src/lib.rs'
    event=compile(name,source,'TEST_CODE_'+name,deps,extra=['--target','x86_64-apple-darwin'])
    # Actual request file spellings; aliases are observed Cargo artifacts, not rewritten argv.
    real=next(v.split('=',1)[1] for v in TEMPLATE if v.startswith(name+'='))
    alias=pathlib.Path(real.format(deps=deps));shutil.copyfile(deps/('lib'+name+'.rmeta'),alias)
    event['filenames']=[str(alias)];emit(event)
for i,name in enumerate(('libring_core_0_17_14_.a','libring_core_0_17_14__test.a')):(out/name).write_bytes(b'TEST_CODE_ARCHIVE_'+str(i).encode())
event={'reason':'build-script-executed','package_id':package,'out_dir':str(out),'linked_libs':libs,'linked_paths':['native='+str(out)],'cfgs':[],'env':[]}
if CASE=='origin_package':event['package_id']='TEST_CODE_OTHER'
if CASE=='origin_outdir':
    other=out.parent/'other';other.mkdir();event['out_dir']=str(other)
if CASE=='linked_libs':event['linked_libs']=list(reversed(libs))
if CASE=='linked_paths':event['linked_paths']=['native='+str(target)]
if CASE=='event_cfg':event['cfgs']=['TEST_CODE']
if CASE=='event_env':event['env']=[['TEST_CODE','1']]
if CASE!='missing_origin':emit(event)
if CASE=='duplicate_origin':emit(event)
argv=[os.environ['RUSTC']]+[v.format(source=root/'src/lib.rs',deps=deps,host=host,out=out) for v in TEMPLATE]
env=dict(os.environ,CARGO_MANIFEST_DIR=str(root),CARGO_MANIFEST_PATH=str(root/'Cargo.toml'),CARGO_PKG_NAME='ring',CARGO_PKG_VERSION='0.17.14',CARGO_PKG_VERSION_MAJOR='0',CARGO_PKG_VERSION_MINOR='17',CARGO_PKG_VERSION_PATCH='14',CARGO_PKG_VERSION_PRE='',CARGO_CRATE_NAME='ring',OUT_DIR=str(out),DYLD_FALLBACK_LIBRARY_PATH=loader,FIXTURE_HIT_ROOT=str(session/'compiler-entry'),RING9_CASE=CASE)
cwd=root
if CASE=='host_probe_control':
    probe_env=dict(env,DYLD_FALLBACK_LIBRARY_PATH=os.environ['DYLD_FALLBACK_LIBRARY_PATH'])
    probe_env.pop('OUT_DIR')
    probe=subprocess.run([os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],'--print','sysroot'],env=probe_env,cwd=root,stdout=subprocess.PIPE)
    if probe.returncode:raise RuntimeError('TEST_CODE genuine ring probe failed')
if CASE=='reorder':argv[-3],argv[-1]=argv[-1],argv[-3]
if CASE=='missing':argv=argv[:-2]
if CASE=='missing_both_libraries':argv=argv[:-4]
if CASE=='missing_all_native':argv=argv[:-6]
if CASE=='attached':argv[-2:]=['-l'+argv[-1]]
if CASE=='dynamic':argv[-1]='dylib=ring_core_0_17_14__test'
if CASE=='extra_native':argv[-6:-6]=['-L','native='+str(out)]
if CASE=='extra_lib':argv+=['-l','static=foreign']
if CASE=='foreign_lib':argv[-1]='static=foreign'
if CASE=='manifest_alias':env['CARGO_MANIFEST_DIR']=str(root.parent)+'//'+name
if CASE=='package':env['CARGO_PKG_NAME']='other'
if CASE=='version':env['CARGO_PKG_VERSION']='0.17.15'
if CASE=='cwd':cwd=app
if CASE=='manifest':env['CARGO_MANIFEST_PATH']=str(app/'Cargo.toml')
if CASE=='feature_missing':i=argv.index('--cfg');del argv[i:i+2]
if CASE=='feature_extra':argv[-6:-6]=['--cfg','feature="other"']
if CASE=='host':i=argv.index('--target');del argv[i:i+2]
if CASE=='wrapper':env['RUSTC_WRAPPER']=str(session/'other-wrapper')
if CASE=='loader':env['DYLD_FALLBACK_LIBRARY_PATH']=loader+':/other'
if CASE=='environment':env['AR']='/TEST_CODE_FOREIGN_AR'
if CASE=='outdir':env['OUT_DIR']=str(target)
if CASE=='link_arg':argv[-6:-6]=['-C','link-arg=-lother']
if CASE=='unknown_flag':argv[-6:-6]=['-Zunknown']
if CASE=='source_arg':argv[4]=str(app/'src/lib.rs')
original=(root/'src/lib.rs').read_bytes()
if CASE=='source_hash':(root/'src/lib.rs').chmod(0o644);(root/'src/lib.rs').write_bytes(b'TEST_CODE_CHANGED')
archive=out/'libring_core_0_17_14_.a'
if CASE=='pre_missing':archive.unlink()
if CASE=='pre_symlink':archive.unlink();archive.symlink_to(root/'src/lib.rs')
if CASE=='pre_hardlink':other=out/'aliased.a';os.link(archive,other)
(session/'ring9-attempt.json').write_text(json.dumps({'argv_hex':[os.fsencode(a).hex() for a in argv]}))
result=subprocess.run([os.environ['RUSTC_WRAPPER'],*argv],env=env,cwd=cwd)
if CASE=='source_hash':(root/'src/lib.rs').write_bytes(original)
if CASE in ('pre_symlink','pre_hardlink'):
    if CASE=='pre_symlink':archive.unlink();archive.write_bytes(b'TEST_CODE_ARCHIVE_0')
    else:other.unlink()
if result.returncode:
    emit({'reason':'build-finished','success':False});sys.exit(result.returncode)
consumer=artifact(package,root/'src/lib.rs','ring','lib',[deps/'libring-caa7295bfcdb1ff4.rmeta',deps/'libring-caa7295bfcdb1ff4.rlib'],features)
if CASE=='consumer_features':consumer['features']=[]
if CASE=='consumer_role':consumer['target']['kind']=['proc-macro']
if CASE=='consumer_source':consumer['target']['src_path']=str(app/'src/lib.rs')
if CASE!='missing_consumer':emit(consumer)
if CASE=='duplicate_consumer':emit(consumer)
app_event=compile('stock_analysis',app/'src/lib.rs','TEST_CODE_app',deps,extra=['--target','x86_64-apple-darwin'])
if CASE=='selected_archive':app_event['filenames']=[str(archive)]
emit(app_event)
paths=list((session/'invocations').glob('*/receipt.json'))
rpath=next(p for p in paths if json.loads(p.read_text()).get('context',{}).get('ring_static_declarations'))
r=json.loads(rpath.read_text())
if CASE=='snapshot_missing':(rpath.parent/r['ring_archive_pre'][0]['snapshot']).unlink()
if CASE=='snapshot_changed':(rpath.parent/r['ring_archive_pre'][0]['snapshot']).write_bytes(b'TEST_CODE_CHANGED')
if CASE=='snapshot_swapped':
    a,b=[rpath.parent/o['snapshot'] for o in r['ring_archive_pre']];old=a.read_bytes();a.write_bytes(b.read_bytes());b.write_bytes(old)
if CASE=='final_archive':archive.write_bytes(b'TEST_CODE_FINAL_CHANGE')
if CASE=='context_tamper':r['context']['out_dir']=str(target);rpath.write_text(json.dumps(r))
if CASE in ('seal_missing_both_libraries','seal_missing_all_native','seal_unannotated_full'):
    trim=4 if CASE=='seal_missing_both_libraries' else (6 if CASE=='seal_missing_all_native' else 0)
    for leaf in ('request.json','invocation.json','receipt.json'):
        p=rpath.parent/leaf;data=json.loads(p.read_text())
        if trim:data['argv_hex']=data['argv_hex'][:-trim]
        if leaf!='request.json':
            data['context']={'kind':'DirectCargoCompile'}
            for key in tuple(data):
                if key.startswith('ring_archive_'):del data[key]
            if trim:
                data['parsed']['options'].pop('-l',None)
                if trim==6:data['parsed']['options']['-L']=[v for v in data['parsed']['options']['-L'] if not v.startswith('native=')]
        p.write_text(json.dumps(data))

if CASE in ('cc_edge','extern_edge','archive_output'):
    p=rpath if CASE!='cc_edge' else next(p for p in paths if json.loads(p.read_text()).get('source')==str(root/'build.rs'))
    data=json.loads(p.read_text())
    if CASE=='archive_output':data['declared_outputs'].append({'path':str(archive),'kind':'link'})
    else:data['externs'][0]['path']=str(target/'absent.rlib')
    p.write_text(json.dumps(data))
emit({'reason':'build-finished','success':True})
'''


# D1 fixtures are synthetic subprocesses, never actual native acquisition.
D1_PROBE = b'#ifdef __clang__\n#pragma message "clang"\n#endif\n\n#ifdef __GNUC__\n#pragma message "gcc"\n#endif\n\n#ifdef __EMSCRIPTEN__\n#pragma message "emscripten"\n#endif\n\n#ifdef __VXWORKS__\n#pragma message "VxWorks"\n#endif\n'
D1_FEATURES = ['bundled','bundled_bindings','cc','default','min_sqlite_version_3_14_0','pkg-config','vcpkg']
D1_NATIVE = r'''
import hashlib,json,os,pathlib,sys
args=sys.argv[1:];role=pathlib.Path(sys.argv[0]).name;case=os.environ.get('D1_CASE','normal')
session=pathlib.Path(os.environ['D1_SESSION']);hits=session/'native-entry';hits.mkdir(exist_ok=True)
ident=str(len(list(hits.iterdir())));entry={'role':role,'args':args,'stdin_eof':sys.stdin.buffer.read()==b'','jobserver':False,'canary_closed':True}
entry['encoded_flags']=os.environ.get('CARGO_ENCODED_RUSTFLAGS')
if 'CARGO_MAKEFLAGS' in os.environ:
    pair=os.environ['CARGO_MAKEFLAGS'].split('--jobserver-fds=')[1].split()[0];r,w=map(int,pair.split(','))
    token=os.read(r,1);os.write(w,token);entry['jobserver']=token==b'J'
if 'D1_CANARY' in os.environ:
    try:os.fstat(int(os.environ['D1_CANARY']));entry['canary_closed']=False
    except OSError:pass
(hits/ident).write_text(json.dumps(entry))
if role=='fake-native-cc':
    if args[0]=='-E':
        source=pathlib.Path(args[-1]);body=source.read_bytes();assert len(body)==206
        if case=='probe_post_missing':source.unlink()
        if case=='probe_post_change':source.write_bytes(body.replace(b'clang',b'CLANG'))
        if case=='probe_retry' and '--' not in args:
            sys.stderr.write('-Wslash-u-filename\n');sys.exit(1)
        sys.stdout.write('clang\n')
    else:
        out=pathlib.Path(args[args.index('-o')+1]);source=pathlib.Path(args[args.index('-c')+1])
        out.write_bytes(b'TEST_CODE_OBJECT:'+source.read_bytes())
        if case=='streams':sys.stdout.buffer.write(b'O'*100000);sys.stderr.buffer.write(b'E'*100000)
        if case=='compile_fail':sys.exit(9)
else:
    mode=args[0];archive=pathlib.Path(args[1])
    if mode in ('cqD','cq'):
        obj=pathlib.Path(args[2]);members=[]
        if archive.exists():members=json.loads(archive.read_text())['members']
        if case in ('fallback','partial_fallback') and mode=='cqD':
            if case=='partial_fallback':archive.write_text(json.dumps({'members':[[obj.name,obj.read_bytes().hex()]],'index':False}))
            sys.stderr.write('TEST_CODE deterministic mode refused\n');sys.exit(3)
        members.append([obj.name,obj.read_bytes().hex()]);archive.write_text(json.dumps({'members':members,'index':False}))
    elif mode in ('sD','s'):
        data=json.loads(archive.read_text())
        if case!='identical_index':data['index']=True;archive.write_text(json.dumps(data))
        if case=='index_fail':sys.exit(8)
    elif mode=='t':
        data=json.loads(archive.read_text());names=[n for n,_ in data['members']]
        if case=='foreign_member':names=['foreign.o']
        if case=='pseudo_member':names=['__.SYMDEF']+names
        if case=='duplicate_member':names+=names
        if case=='inspector_diagnostic':sys.stderr.write('unreviewed diagnostic\n')
        sys.stdout.write('\n'.join(names)+'\n')
    elif mode=='p':
        data=json.loads(archive.read_text());body=bytes.fromhex(next(v for n,v in data['members'] if n==args[2]))
        if case=='extract_change':body=b'X'+body[1:]
        if case=='extract_overflow':body+=b'X'
        sys.stdout.buffer.write(body)
        if case=='inspector_archive_change':archive.write_bytes(b'TEST_CODE_DRIFT')
    else:raise AssertionError(args)
'''
D1_BUILDER = r'''
import json,os,pathlib,subprocess,sys
root=pathlib.Path.cwd();out=pathlib.Path(os.environ['OUT_DIR']);session=pathlib.Path(os.environ['D1_SESSION']);case=os.environ['D1_CASE']
env=dict(os.environ);probe=out/'42detect_compiler_family.c';probe.write_bytes((session/'vendor/cc/src/detect_compiler_family.c').read_bytes())
if case=='probe_literal':probe.write_bytes(b'X'+probe.read_bytes()[1:])
if case=='probe_overflow':probe.write_bytes(probe.read_bytes()+b'X')
if case=='probe_path':probe.rename(out/'042detect_compiler_family.c');probe=out/'042detect_compiler_family.c'
if case=='package':env['CARGO_PKG_NAME']='ring'
if case=='override':env['CFLAGS_x86_64-apple-darwin']='-ffast-math'
if case=='host_override':env['HOST_CC']='/other'
if case=='encoded_flags':env['CARGO_ENCODED_RUSTFLAGS']='-Clink-arg=foreign'
if case=='target_flags':env['CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS']=''
if case=='loader':env['DYLD_FALLBACK_LIBRARY_PATH']+=':/other'
r,w=os.pipe();os.write(w,b'J');canary=os.open(root/'Cargo.toml',os.O_RDONLY);pair=(r,w)
env['CARGO_MAKEFLAGS']=f'-j --jobserver-fds={r},{w} --jobserver-auth={r},{w}'
env['D1_CANARY']=str(canary)
if case=='fd_reversed':env['CARGO_MAKEFLAGS']=f'-j --jobserver-fds={w},{r} --jobserver-auth={w},{r}'
if case=='fd_foreign':
    r2,w2=os.pipe();pair=(r,w2);env['CARGO_MAKEFLAGS']=f'-j --jobserver-fds={r},{w2} --jobserver-auth={r},{w2}'
if case=='fd_closed':os.close(r);pair=(w,)
def run(role,args):
    attempt={'role':role,'args_hex':[os.fsencode(a).hex() for a in args]}
    with open(session/'native-attempts.jsonl','a') as f:f.write(json.dumps(attempt)+'\n')
    result=subprocess.run([env[role],*args],env=env,pass_fds=(*pair,canary),stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    with open(session/'builder-native-results.jsonl','a') as f:f.write(json.dumps({'code':result.returncode,'stdout':result.stdout.hex(),'stderr':result.stderr.hex()})+'\n')
    return result
p=run('CC',['-E',str(probe)])
if case=='probe_retry':p=run('CC',['-E','--',str(probe)])
if case=='unauthorized_retry':p=run('CC',['-E','--',str(probe)])
if p.returncode:
    if case=='capture_fault':pass
    else:sys.exit(p.returncode)
if case not in ('probe_survives','probe_consumed','probe_relative_consumed') and probe.exists():probe.unlink()
obj=out/'sqlite3-test.o'
ccargs=['-O0','-DSQLITE_CORE','-c','sqlite3/sqlite3.c','-o',str(obj)]
if case=='compile_plugin':ccargs.insert(0,'-fplugin=/other')
if case=='compile_source':ccargs[ccargs.index('-c')+1]='build.rs'
c=run('CC',ccargs)
if c.returncode:sys.exit(c.returncode)
archive=out/'libsqlite3.a'
if case=='archive_initial':archive.write_bytes(b'TEST_CODE_PREEXISTING')
env['ZERO_AR_DATE']='1'
a=run('AR',['cqD',str(archive),str(obj)])
mode='sD'
if a.returncode:
    a=run('AR',['cq',str(archive),str(obj)]);mode='s'
if a.returncode:sys.exit(a.returncode)
if case=='predecessor_change':archive.write_bytes(b'TEST_CODE_REPLACED')
i=run('AR',[mode,str(archive)])
if i.returncode:sys.exit(i.returncode)
if case=='extra_index':
    result=run('AR',[mode,str(archive)])
    if result.returncode:sys.exit(result.returncode)
(out/'bindgen.rs').write_bytes((root/'sqlite3/bindgen_bundled_version.rs').read_bytes())
if case=='object_drift':obj.write_bytes(b'TEST_CODE_OBJECT_DRIFT')
if case=='archive_drift':archive.write_bytes(b'TEST_CODE_ARCHIVE_DRIFT')
if case=='bindings_drift':(out/'bindgen.rs').write_bytes(b'TEST_CODE_BINDINGS_DRIFT')
if case=='snapshot_drift':
    file=next((session/'native-invocations').glob('*/input-pre.raw'));file.write_bytes(b'TEST_CODE_SNAPSHOT_DRIFT')
os.close(w);os.close(canary)
if case!='fd_closed':os.close(r)
'''
D1_RUSTC = r'''
import json,os,pathlib,sys
args=sys.argv[1:]
def val(k):return next((a.split('=',1)[1] for a in args if a.startswith(k+'=')),None) or args[args.index(k)+1]
name=val('--crate-name');source=next(pathlib.Path(a) for a in args if a.endswith('.rs'));out=pathlib.Path(val('--out-dir'));out.mkdir(parents=True,exist_ok=True)
hits=pathlib.Path(os.environ['D1_SESSION'])/'rust-entry';hits.mkdir(exist_ok=True);(hits/name).write_text(json.dumps(args))
files=[out/name] if val('--crate-type')=='bin' else [out/('lib'+name+'.rlib'),out/('lib'+name+'.rmeta')]
for f in files:
    if name=='build_script_sqlite':f.write_text('#!'+sys.executable+' -I\n'+__BUILDER__);f.chmod(0o700)
    else:f.write_bytes(b'TEST_CODE_RUST:'+source.read_bytes())
def esc(s):return s.replace(chr(92),chr(92)*2).replace(' ',chr(92)+' ').replace('#',chr(92)+'#').replace(':',chr(92)+':').replace('$','$$')
inputs=[str(source)]
if name=='libsqlite3_sys':inputs.append(str(pathlib.Path(os.environ['OUT_DIR'])/'bindgen.rs'))
if name=='libsqlite3_sys':
    case=os.environ['D1_CASE'];outdir=pathlib.Path(os.environ['OUT_DIR'])
    member={'probe_consumed':'42detect_compiler_family.c','probe_relative_consumed':'42detect_compiler_family.c',
            'object_relative_consumed':'sqlite3-test.o','archive_relative_consumed':'libsqlite3.a'}.get(case)
    if member:
        path=str(outdir/member)
        inputs.append(os.path.relpath(path,pathlib.Path.cwd()) if 'relative' in case else path)

(out/(name+'.d')).write_text(esc(str(files[0]))+': '+' '.join(map(esc,inputs))+'\n')
print(json.dumps({'fixture_argv':args}),file=sys.stderr)
'''
D1_CARGO = r'''
import json,os,pathlib,shutil,subprocess,sys
CASE=__CASE__;FEATURES=__FEATURES__;argv=sys.argv[1:]
assert argv[-2:]==['--features','replay-sqlite-bundled-v1']
app=pathlib.Path(argv[argv.index('--manifest-path')+1]).parent;session=app.parent;target=session/'target';host=target/'debug/deps';deps=target/'x86_64-apple-darwin/debug/deps'
root=session/'vendor/libsqlite3-sys';out=target/'x86_64-apple-darwin/debug/build/libsqlite3-sys-0123456789abcdef/out';out.mkdir(parents=True)
package='registry+https://github.com/rust-lang/crates.io-index#libsqlite3-sys@0.28.0'
cfg=[v for f in FEATURES for v in ('--cfg','feature="'+f+'"')]
base=dict(os.environ,D1_CASE=CASE,D1_SESSION=str(session),CARGO_ENCODED_RUSTFLAGS='')
loader=str(host)+':'+os.environ['DYLD_FALLBACK_LIBRARY_PATH']
def emit(e):print(json.dumps(e),flush=True)
def compile(name,source,pkg,dest,kind='lib',extra=(),feats=()):
    env=dict(base,CARGO_MANIFEST_DIR=str(source.parent if source.name=='build.rs' else source.parent.parent),CARGO_MANIFEST_PATH=str(source.parent/'Cargo.toml' if source.name=='build.rs' else source.parent.parent/'Cargo.toml'),DYLD_FALLBACK_LIBRARY_PATH=loader)
    if pkg==package:env.update(CARGO_PKG_NAME='libsqlite3-sys',CARGO_PKG_VERSION='0.28.0',OUT_DIR=str(out))
    args=['--crate-name',name,'--edition=2021',str(source),'--crate-type',kind,'--emit='+('dep-info,link' if kind=='bin' else 'dep-info,metadata,link'),'--out-dir',str(dest),*extra]
    p=subprocess.run([env['RUSTC_WRAPPER'],env['RUSTC'],*args],env=env,cwd=env['CARGO_MANIFEST_DIR'])
    if p.returncode:emit({'reason':'build-finished','success':False});sys.exit(p.returncode)
    files=[dest/name] if kind=='bin' else [dest/('lib'+name+'.rlib'),dest/('lib'+name+'.rmeta')]
    event={'reason':'compiler-artifact','package_id':pkg,'target':{'src_path':str(source),'name':name,'kind':['custom-build' if kind=='bin' else 'lib'],'crate_types':[kind]},'features':list(feats),'filenames':list(map(str,files)),'executable':None,'fresh':False}
    if kind=='bin':
        alias=dest/'build-script-build';shutil.copyfile(files[0],alias);alias.chmod(0o700);event['filenames']=[str(alias)]
    if not (CASE=='missing_builder' and kind=='bin'):emit(event)
    return files[0]
cc=compile('cc',session/'vendor/cc/src/lib.rs','registry+https://github.com/rust-lang/crates.io-index#cc@1.2.59',host)
builder=compile('build_script_sqlite',root/'build.rs',package,target/'debug/build/libsqlite3-sys-fedcba9876543210','bin',cfg+['--extern','cc='+str(host/'libcc.rlib')],FEATURES)
sysroot=pathlib.Path(os.environ['DYLD_FALLBACK_LIBRARY_PATH']).parent
native_loader=':'.join(map(str,(target/'debug',host,sysroot/'lib/rustlib/x86_64-apple-darwin/lib',sysroot/'lib')))
env=dict(base,CARGO_MANIFEST_DIR=str(root),CARGO_MANIFEST_PATH=str(root/'Cargo.toml'),CARGO_PKG_NAME='libsqlite3-sys',CARGO_PKG_VERSION='0.28.0',OUT_DIR=str(out),HOST='x86_64-apple-darwin',TARGET='x86_64-apple-darwin',OPT_LEVEL='0',DEBUG='true',DYLD_FALLBACK_LIBRARY_PATH=native_loader)
env.update({'CARGO_FEATURE_'+f.upper().replace('-','_'):'1' for f in FEATURES})
p=subprocess.run([str(builder)],cwd=root,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
(session/'builder.stdout.raw').write_bytes(p.stdout);(session/'builder.stderr.raw').write_bytes(p.stderr)
if p.returncode:emit({'reason':'build-finished','success':False});sys.exit(p.returncode)
event={'reason':'build-script-executed','package_id':package,'out_dir':str(out),'linked_libs':['static=sqlite3'],'linked_paths':['native='+str(out)],'cfgs':[],'env':[]}
if CASE=='origin_outdir':event['out_dir']=str(out.parent/'other')
if CASE=='link_directive':event['linked_libs']=['dylib=sqlite3']
if CASE!='missing_origin':emit(event)
if CASE=='duplicate_origin':emit(event)
extra=cfg+['--target','x86_64-apple-darwin','-L','dependency='+str(deps),'-L','dependency='+str(host),'-L','native='+str(out),'-l','static=sqlite3']
if CASE=='rust_missing_static':extra=extra[:-2]
compile('libsqlite3_sys',root/'src/lib.rs',package,deps,extra=extra,feats=FEATURES)
for name,version in [('diesel','2.3.7'),('rusqlite','0.31.0')]:
    compile(name,session/'vendor'/name/'src/lib.rs','registry+https://github.com/rust-lang/crates.io-index#'+name+'@'+version,deps,extra=['--target','x86_64-apple-darwin','--extern','libsqlite3_sys='+str(deps/'liblibsqlite3_sys.rlib')])
compile('stock_analysis',app/'src/lib.rs','TEST_CODE_app',deps,extra=['--target','x86_64-apple-darwin','--extern','rusqlite='+str(deps/'librusqlite.rlib'),'--extern','diesel='+str(deps/'libdiesel.rlib')])
emit({'reason':'build-finished','success':True})
'''

# Deterministic protocol fixtures, not real rustversion/thiserror or Cargo.
RECORD10_RUSTC = r'''
import json,os,pathlib,signal,sys
args=sys.argv[1:];case=os.environ.get('RECORD10_CASE','normal')
hits=pathlib.Path(os.environ['FIXTURE_HIT_ROOT']);hits.mkdir(parents=True,exist_ok=True)
def value(k):
    inline=[a.split('=',1)[1] for a in args if a.startswith(k+'=')]
    return inline[0] if inline else args[args.index(k)+1]
version=args==['--version'];probe='build/probe.rs' in args
name='version' if version else ('feature-probe' if probe else value('--crate-name'))
stdout=b'rustc 1.95.0 (TEST_CODE protocol oracle)\n' if version else b'TEST_CODE_stream_out\n'
stderr=(json.dumps({'fixture_argv':args,'fixture_environment':dict(os.environ)},sort_keys=True)+'\n').encode()
(hits/(name+'-'+os.environ.get('CARGO_PKG_NAME','application')+'.json')).write_text(json.dumps({'argv':sys.argv,'environment':dict(os.environ),'cwd':str(pathlib.Path.cwd()),'stdout_hex':stdout.hex(),'stderr_hex':stderr.hex()}))
os.write(1,stdout);os.write(2,stderr)
if version:sys.exit(1 if case=='version-fail' else 0)
source=next(pathlib.Path(a) for a in args if a.endswith('.rs'))
out=pathlib.Path(value('--out-dir'));out.mkdir(parents=True,exist_ok=True)
crate=value('--crate-name');kind=value('--crate-type')
def esc(s):return str(s).replace(chr(92),chr(92)*2).replace(' ',chr(92)+' ').replace('#',chr(92)+'#').replace(':',chr(92)+':').replace('$','$$')
deps=[source.resolve()]
if not probe and source.name=='build.rs' and os.environ.get('CARGO_PKG_NAME')=='rustversion':deps.append(source.parent/'rustc.rs')
if not probe and source.name=='lib.rs' and os.environ.get('CARGO_PKG_NAME') in ('rustversion','thiserror'):
    deps.append(pathlib.Path(os.environ['OUT_DIR'])/('version.expr' if os.environ['CARGO_PKG_NAME']=='rustversion' else 'private.rs'))
if probe:
    code=0 if case.startswith('probe0') else (2 if case=='probe2' else 1)
    variant='both' if case in ('snapshot-missing','snapshot-tamper') else (case.split('-',1)[1] if case.startswith(('probe0-','probe1-')) else 'none')
    files=[]
    if variant in ('both','dep'):(out/'thiserror.d').write_text('out: '+esc(source.resolve())+'\n');files.append(out/'thiserror.d')
    if variant in ('both','meta'):(out/'libthiserror.rmeta').write_bytes(b'TEST_CODE_partial_metadata');files.append(out/'libthiserror.rmeta')
    if case=='dep-error':(out/'thiserror.d').write_text('malformed TEST_CODE dep-info')
    if case=='capture-alias':(out/'thiserror.d').symlink_to(source.resolve())
    if case=='capture-late':
        (out/'thiserror.d').write_text('out: '+esc(source.resolve())+'\n');(out/'libthiserror.rmeta').symlink_to(source.resolve())
    if case=='source-post':source.chmod(0o644);source.write_bytes(b'TEST_CODE_post_child_source_drift');source.chmod(0o444)
    if case=='signal':os.kill(os.getpid(),signal.SIGKILL)
    sys.exit(code)
files=[out/crate] if kind=='bin' else ([out/('lib'+crate+'.rmeta'),out/('lib'+crate+'.dylib')] if kind=='proc-macro' else [out/('lib'+crate+'.rmeta'),out/('lib'+crate+'.rlib')])
for f in files:f.write_bytes(b'TEST_CODE_output:'+crate.encode()+b':'+source.read_bytes())
(out/(crate+'.d')).write_text(esc(files[0])+': '+' '.join(esc(p) for p in deps)+'\n')
'''

RECORD10_CARGO = r'''
import hashlib,json,os,pathlib,shutil,subprocess,sys
CASE=__CASE__
args=sys.argv[1:]
def value(k):return args[args.index(k)+1]
app=pathlib.Path(value('--manifest-path')).parent;session=app.parent;target=pathlib.Path(value('--target-dir'))
host=target/'debug/deps';deps=target/'x86_64-apple-darwin/debug/deps'
for p in (host,deps):p.mkdir(parents=True,exist_ok=True)
loader=str(host)+':'+os.environ['DYLD_FALLBACK_LIBRARY_PATH']
nested=str(target/'debug')+':'+str(host)+':'+str(pathlib.Path(os.environ['RUSTC']).parent/'sysroot/lib/rustlib/x86_64-apple-darwin/lib')+':'+os.environ['DYLD_FALLBACK_LIBRARY_PATH']
def emit(e):print(json.dumps(e),flush=True)
def artifact(pkg,source,name,kind,files,features):
    manifest=app/'Cargo.toml' if pkg=='TEST_CODE_app' else session/'vendor'/pkg.rsplit('#',1)[1].split('@')[0]/'Cargo.toml'
    return {'reason':'compiler-artifact','package_id':pkg,'manifest_path':str(manifest),'target':{'src_path':str(source),'kind':[kind],'crate_types':['bin' if kind=='custom-build' else kind],'name':name,'edition':'2021'},'features':features,'filenames':[str(p) for p in files],'executable':None,'fresh':False}
def env_for(name,version,root,out):
    major,minor,patch=version.split('.')
    env=dict(os.environ,CARGO_PKG_NAME=name,CARGO_PKG_VERSION=version,CARGO_PKG_VERSION_MAJOR=major,CARGO_PKG_VERSION_MINOR=minor,CARGO_PKG_VERSION_PATCH=patch,CARGO_PKG_VERSION_PRE='',CARGO_MANIFEST_DIR=str(root),CARGO_MANIFEST_PATH=str(root/'Cargo.toml'),OUT_DIR=str(out),HOST='x86_64-apple-darwin',TARGET='x86_64-apple-darwin',CARGO_ENCODED_RUSTFLAGS='',FIXTURE_HIT_ROOT=str(session/'compiler-entry'),RECORD10_CASE=CASE,DYLD_FALLBACK_LIBRARY_PATH=loader)
    if name=='thiserror':env.update(CARGO_FEATURE_DEFAULT='1',CARGO_FEATURE_STD='1')
    return env
def compile(name,source,pkg,dest,kind,env,cfg=()):
    command=[os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],'--crate-name',name,'--edition=2021',str(source),'--crate-type',kind,'--emit='+('dep-info,link' if kind=='bin' else 'dep-info,metadata,link'),'--out-dir',str(dest)]+[v for c in cfg for v in ('--cfg',c)]
    if kind=='lib':command+=['--target','x86_64-apple-darwin']
    execution_cwd=pathlib.Path(env['CARGO_MANIFEST_DIR'])
    if env.get('CARGO_PKG_NAME')=='thiserror' and ((CASE=='consumer-cwd' and kind=='lib') or (CASE=='builder-cwd' and kind=='bin')):execution_cwd=app
    result=subprocess.run(command,env=env,cwd=execution_cwd,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    if result.returncode:raise RuntimeError(result.stderr.decode(errors='replace'))
    files=[dest/name] if kind=='bin' else ([dest/('lib'+name+'.rmeta'),dest/('lib'+name+'.dylib')] if kind=='proc-macro' else [dest/('lib'+name+'.rmeta'),dest/('lib'+name+'.rlib')])
    return artifact(pkg,source,name,'custom-build' if kind=='bin' else kind,files,[] if env.get('CARGO_PKG_NAME')!='thiserror' else ['default','std'])
for name,version in (('rustversion','1.0.22'),('thiserror','2.0.18')):
    root=session/'vendor'/name;pkg='registry+https://github.com/rust-lang/crates.io-index#'+name+'@'+version
    out=(target/'debug/build' if name=='rustversion' else target/'x86_64-apple-darwin/debug/build')/(name+'-0123456789abcdef')/'out';out.mkdir(parents=True)
    env=env_for(name,version,root,out);cfg=[] if name=='rustversion' else ['feature="default"','feature="std"']
    if CASE=='ordinary-probe' and name=='rustversion':
        ordinary=dict(env,DYLD_FALLBACK_LIBRARY_PATH=os.environ['DYLD_FALLBACK_LIBRARY_PATH']);ordinary.pop('OUT_DIR')
        result=subprocess.run([os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],'--version'],env=ordinary,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        if result.returncode:raise RuntimeError('TEST_CODE ordinary Cargo probe refused')
    source=root/('build/build.rs' if name=='rustversion' else 'build.rs')
    builderenv={k:v for k,v in env.items() if not k.startswith('CARGO_FEATURE_') and k not in ('OUT_DIR','HOST','TARGET','CARGO_ENCODED_RUSTFLAGS')}
    builder=compile('build_script_build',source,pkg,target/'debug/build'/(name+'-fedcba9876543210'),'bin',builderenv,cfg)
    original=pathlib.Path(builder['filenames'][0]);alias=original.parent/'build-script-build';shutil.copyfile(original,alias);builder['filenames']=[str(alias)]
    if CASE=='alias-bytes' and name=='thiserror':alias.write_bytes(b'TEST_CODE_wrong_alias')
    if CASE!='missing-builder' or name!='thiserror':emit(builder)
    if CASE=='duplicate-builder' and name=='thiserror':emit(builder)
    if CASE=='builder-raw' and name=='thiserror':
        p=next(p for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('source')==str(source));request=p.parent/'request.json';r=json.loads(request.read_text())
        r['argv_hex']=[os.fsencode(str(root/'src/lib.rs')).hex() if a==os.fsencode(str(source)).hex() else a for a in r['argv_hex']];request.write_text(json.dumps(r))
    if CASE=='builder-request-cwd' and name=='thiserror':
        p=next(p for p in (session/'invocations').glob('*/request.json') if (p.parent/'receipt.json').exists() and json.loads((p.parent/'receipt.json').read_text()).get('source')==str(source));r=json.loads(p.read_text());r['cwd_hex']=os.fsencode(str(app)).hex();p.write_text(json.dumps(r))
    if name=='thiserror':(out/'private.rs').write_bytes(b'#[doc(hidden)]\npub mod __private18 {\n    #[doc(hidden)]\n    pub use crate::private::*;\n}\n')
    raw=[os.environ['RUSTC'],'--version'] if name=='rustversion' else [os.environ['RUSTC'],'--edition=2018','--crate-name=thiserror','--crate-type=lib','--cap-lints=allow','--emit=dep-info,metadata','--out-dir',str(out/'probe'),'build/probe.rs','--target','x86_64-apple-darwin']
    childenv=dict(env,DYLD_FALLBACK_LIBRARY_PATH=nested);cwd=root;saved={};variant=None
    if CASE.startswith('reject:'+name+':'):
        variant=CASE.split(':',2)[2]
        if variant=='version':childenv['CARGO_PKG_VERSION']='0.0.1'
        if variant=='components':childenv['CARGO_PKG_VERSION_PATCH']='99'
        if variant=='package':childenv['CARGO_PKG_NAME']='unknown'
        if variant=='manifest':childenv['CARGO_MANIFEST_PATH']=str(app/'Cargo.toml')
        if variant=='cwd':cwd=app
        if variant=='features':childenv['CARGO_FEATURE_UNKNOWN']='1'
        if variant=='feature-missing':childenv.pop('CARGO_FEATURE_STD',None)
        if variant=='host':childenv['HOST']='unknown'
        if variant=='target-env':childenv['TARGET']='unknown'
        if variant=='outdir':childenv['OUT_DIR']=str(target)
        if variant=='wrapper':childenv['RUSTC_WRAPPER']=str(session/'other')
        if variant=='compiler':childenv['RUSTC']=str(session/'other')
        if variant=='loader':childenv['DYLD_FALLBACK_LIBRARY_PATH']=nested+':/other'
        if variant=='bootstrap':childenv['RUSTC_BOOTSTRAP']=''
        if variant=='stage':childenv['RUSTC_STAGE']=''
        if variant=='workspace':childenv['RUSTC_WORKSPACE_WRAPPER']='/other'
        if variant=='workspace-empty':childenv['RUSTC_WORKSPACE_WRAPPER']=''
        if variant=='encoded':childenv['CARGO_ENCODED_RUSTFLAGS']='-ZTEST_CODE'
        if variant in ('source','symlink','hardlink','directory','manifest-hash'):
            leaf=root/('Cargo.toml' if variant=='manifest-hash' else ('build/rustc.rs' if name=='rustversion' else 'build/probe.rs'));saved[leaf]=leaf.read_bytes();leaf.unlink()
            if variant=='symlink':leaf.symlink_to(root/'src/lib.rs')
            elif variant=='hardlink':os.link(root/'src/lib.rs',leaf)
            elif variant=='directory':leaf.mkdir()
            else:leaf.write_bytes(b'TEST_CODE_changed')
        if variant=='empty':raw=raw[:1]
        if variant=='retry':raw=[raw[0],'--rustc','--version']
        if variant=='extra':raw+=['--test']
        if variant=='native':raw+=['-l','TEST_CODE']
        if variant=='extern':raw+=['--extern','TEST_CODE='+str(host/'foreign.rlib')]
        if variant=='reorder':raw=raw[:1]+list(reversed(raw[1:]))
        if variant=='wrong-source':raw=[a.replace('build/probe.rs','build/other.rs') for a in raw]
        if variant=='missing-target':raw=raw[:-2]
        if variant=='missing-group':raw=raw[:1]+raw[-1:]
    if CASE.startswith('namespace-noreceipt-') and name=='thiserror':childenv.pop('CARGO_FEATURE_STD')
    (session/(name+'-attempt.json')).write_text(json.dumps({'argv_hex':[os.fsencode(a).hex() for a in raw],'environment_hex':{os.fsencode(k).hex():os.fsencode(v).hex() for k,v in childenv.items()},'cwd_hex':os.fsencode(str(cwd)).hex()}))
    zero=CASE=='zero-child' and name=='thiserror'
    source_saved=(root/'build/probe.rs').read_bytes() if CASE=='source-post' and name=='thiserror' else None
    result=None if zero else subprocess.run([os.environ['RUSTC_WRAPPER'],*raw],env=childenv,cwd=cwd,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    if CASE=='duplicate-child' and name=='thiserror':subprocess.run([os.environ['RUSTC_WRAPPER'],*raw],env=childenv,cwd=cwd,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    for leaf,b in saved.items():
        if leaf.is_dir() and not leaf.is_symlink():leaf.rmdir()
        else:leaf.unlink()
        leaf.write_bytes(b)
    if CASE=='source-post' and name=='thiserror':
        leaf=root/'build/probe.rs';leaf.chmod(0o644);leaf.write_bytes(source_saved);leaf.chmod(0o444)
    if name=='rustversion':(out/'version.expr').write_bytes(b'TEST_CODE_VERSION_EXPR:'+((result.stdout if result else b'')))
    else:
        probe=out/'probe'
        retained=[]
        for p in (session/'invocations').glob('*/receipt.json'):
            r=json.loads(p.read_text())
            if r.get('context',{}).get('kind')=='ThiserrorStaticFeatureProbe':retained.append({'id':p.parent.name,'code':r['exit_code'],'outputs':r['outputs']})
        (session/'before-builder-cleanup.json').write_text(json.dumps(retained))
        if probe.exists():shutil.rmtree(probe)
    for p in (session/'invocations').glob('*/receipt.json'):
        r=json.loads(p.read_text())
        if name=='thiserror' and r.get('context',{}).get('kind')=='ThiserrorStaticFeatureProbe':
            if CASE in ('snapshot-missing','snapshot-tamper') and r['outputs']:
                q=p.parent/r['outputs'][0]['snapshot'];q.unlink() if CASE=='snapshot-missing' else q.write_bytes(b'TEST_CODE_tamper')
            if CASE.startswith('namespace-') and not CASE.startswith('namespace-noreceipt-'):
                denial=CASE.split('-')[1];request=p.parent/'request.json';q=json.loads(request.read_text())
                if denial=='features':q['environment_hex'].pop(b'CARGO_FEATURE_STD'.hex())
                elif denial=='manifest':q['environment_hex'][b'CARGO_MANIFEST_PATH'.hex()]=os.fsencode(str(app/'Cargo.toml')).hex()
                elif denial=='template':q['argv_hex'].remove(b'--cap-lints=allow'.hex())
                else:raise RuntimeError('TEST_CODE unknown fixed namespace denial')
                request.write_text(json.dumps(q));r['environment_hex']=q['environment_hex'];r['argv_hex']=q['argv_hex']
                initial=p.parent/'invocation.json';i=json.loads(initial.read_text());i['environment_hex']=q['environment_hex'];i['argv_hex']=q['argv_hex'];initial.write_text(json.dumps(i))
            if CASE=='unannotated' or CASE.startswith('namespace-'):
                r['context']={'kind':'DirectCargoCompile'};r['kind']='Compile';p.write_text(json.dumps(r))
                initial=p.parent/'invocation.json';i=json.loads(initial.read_text());i['context']=r['context'];i['kind']='Compile';initial.write_text(json.dumps(i))
    code=0 if zero else result.returncode
    cfgs=['error_generic_member_access'] if name=='thiserror' and code==0 else []
    if CASE=='cfg-mismatch' and name=='thiserror':cfgs=[] if cfgs else ['error_generic_member_access']
    event={'reason':'build-script-executed','package_id':pkg,'out_dir':str(out),'linked_libs':[],'linked_paths':[],'cfgs':cfgs,'env':[]}
    if CASE=='origin-package' and name=='thiserror':event['package_id']='TEST_CODE_other'
    if CASE=='origin-outdir' and name=='thiserror':event['out_dir']=str(out.parent/'other');pathlib.Path(event['out_dir']).mkdir()
    if CASE!='missing-origin' or name!='thiserror':emit(event)
    if CASE=='duplicate-origin' and name=='thiserror':emit(event)
    if variant:continue
    if CASE=='private-bytes' and name=='thiserror':(out/'private.rs').write_bytes(b'TEST_CODE_wrong_private')
    consumerenv={k:v for k,v in env.items() if not k.startswith('CARGO_FEATURE_') and k not in ('HOST','TARGET','CARGO_ENCODED_RUSTFLAGS')}
    lib=compile(name,root/'src/lib.rs',pkg,host if name=='rustversion' else deps,'proc-macro' if name=='rustversion' else 'lib',consumerenv,cfg+cfgs)
    if CASE=='consumer-initial-cwd' and name=='thiserror':
        p=next(p.parent/'invocation.json' for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('source')==str(root/'src/lib.rs'));i=json.loads(p.read_text());i['cwd']=str(app);p.write_text(json.dumps(i))
    if CASE!='missing-consumer' or name!='thiserror':emit(lib)
    if CASE=='duplicate-consumer' and name=='thiserror':emit(lib)
    if CASE=='generated-dep-missing' and name=='thiserror':
        (deps/'thiserror.d').write_text('out: '+str(root/'src/lib.rs')+'\n')
        p=next(p for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('source')==str(root/'src/lib.rs'));r=json.loads(p.read_text())
        for o in r['outputs']:
            if o['kind']=='dep-info':o['dep_info']['paths']=[str(root/'src/lib.rs')];o['sha256']=hashlib.sha256((deps/'thiserror.d').read_bytes()).hexdigest()
        p.write_text(json.dumps(r))
    if CASE in ('transient-artifact','transient-extern','transient-consumed','transient-ordinary') and name=='thiserror':
        forbidden=out/'probe/thiserror.d';forbidden.parent.mkdir(exist_ok=True);forbidden.write_bytes(b'TEST_CODE_recreated')
        if CASE=='transient-artifact':emit(artifact(pkg,root/'src/lib.rs','thiserror','lib',[forbidden],['default','std']))
        if CASE=='transient-extern':
            p=next(p for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('source')==str(root/'src/lib.rs'));r=json.loads(p.read_text());r['externs']=[{'name':'TEST_CODE','path':str(forbidden)}];p.write_text(json.dumps(r))
        if CASE=='transient-consumed':
            (deps/'thiserror.d').write_text('out: '+str(forbidden)+'\n')
            p=next(p for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('source')==str(root/'src/lib.rs'));r=json.loads(p.read_text())
            for o in r['outputs']:
                if o['kind']=='dep-info':o['dep_info']['paths']=[str(forbidden)]
            p.write_text(json.dumps(r))
        if CASE=='transient-ordinary':
            p=next(p for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('source')==str(root/'src/lib.rs'));r=json.loads(p.read_text());r['declared_outputs'].append({'path':str(forbidden),'kind':'metadata'});p.write_text(json.dumps(r))
    if CASE.startswith('namespace-') and name=='thiserror':
        _,denial,reuse,leaf=CASE.split('-');filename='thiserror.d' if leaf=='dep' else 'libthiserror.rmeta'
        forbidden=out/'probe'/filename;forbidden.parent.mkdir(exist_ok=True);forbidden.write_bytes(b'TEST_CODE_combined_recreated')
        p=next(p for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('source')==str(root/'src/lib.rs'));r=json.loads(p.read_text())
        if reuse=='artifact':emit(artifact(pkg,root/'src/lib.rs','thiserror','lib',[forbidden],['default','std']))
        elif reuse=='extern':r['externs']=[{'name':'TEST_CODE','path':str(forbidden)}];p.write_text(json.dumps(r))
        elif reuse=='consumed':
            dep=deps/'thiserror.d';dep.write_text('out: '+str(forbidden)+'\n')
            for o in r['outputs']:
                if o['kind']=='dep-info':o['dep_info']['paths']=[str(forbidden)];o['sha256']=hashlib.sha256(dep.read_bytes()).hexdigest()
            p.write_text(json.dumps(r))
        elif reuse=='ordinary':
            r['declared_outputs'].append({'path':str(forbidden),'kind':'metadata'})
            r['outputs'].append({'path':str(forbidden),'kind':'metadata','sha256':hashlib.sha256(forbidden.read_bytes()).hexdigest()});p.write_text(json.dumps(r))
            emit(artifact('TEST_CODE_app',app/'src/lib.rs','stock_analysis','lib',[forbidden],[]))
        else:raise RuntimeError('TEST_CODE unknown fixed namespace reuse')
    if CASE=='version-owned-file' and name=='rustversion':
        p=next(p for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('context',{}).get('kind')=='RustversionBuildVersion');r=json.loads(p.read_text());r['outputs']=[{'path':str(out/'version.expr'),'kind':'metadata','sha256':'0'*64}];p.write_text(json.dumps(r))
env=dict(os.environ,CARGO_MANIFEST_DIR=str(app),DYLD_FALLBACK_LIBRARY_PATH=loader,FIXTURE_HIT_ROOT=str(session/'compiler-entry'))
emit(compile('stock_analysis',app/'src/lib.rs','TEST_CODE_app',deps,'lib',env))
emit({'reason':'build-finished','success':CASE!='cargo-failure'})
'''


PSM11_ARGS = ['--crate-name', 'psm', '--edition=2021', '{source}', '--error-format=json', '--json=diagnostic-rendered-ansi,artifacts,future-incompat', '--crate-type', 'lib', '--emit=dep-info,metadata,link', '-C', 'embed-bitcode=no', '-C', 'debuginfo=1', '-C', 'split-debuginfo=unpacked', '--check-cfg', 'cfg(docsrs,test)', '--check-cfg', 'cfg(feature, values())', '-C', 'metadata=85deb609eb7b9709', '-C', 'extra-filename=-749f77748b047fe7', '--out-dir', '{deps}', '--target', 'x86_64-apple-darwin', '-L', 'dependency={deps}', '-L', 'dependency={host}', '--cap-lints', 'allow', '-L', 'native={out}', '-l', 'static=psm_s', '--cfg', 'asm', '--cfg', 'link_asm', '--cfg', 'switchable_stack', '--check-cfg', 'cfg(switchable_stack,asm,link_asm)']

PSM11_RUSTC = r'''
import json,os,pathlib,sys
args=sys.argv[1:]
def value(k):
    inline=[a.split('=',1)[1] for a in args if a.startswith(k+'=')]
    return inline[0] if inline else args[args.index(k)+1]
name=value('--crate-name');source=next(pathlib.Path(a) for a in args if a.endswith('.rs'))
hits=pathlib.Path(os.environ['FIXTURE_HIT_ROOT']);hits.mkdir(parents=True,exist_ok=True)
(hits/('compile-'+name)).write_text(json.dumps(args));case=os.environ.get('PSM11_CASE','normal')
if name=='psm':
    archive=pathlib.Path(os.environ['OUT_DIR'])/'libpsm_s.a'
    if case=='during_change':archive.write_bytes(b'TEST_CODE_CHANGED_DURING_CHILD')
    if case=='post_missing':archive.unlink()
    if case=='post_symlink':archive.unlink();archive.symlink_to(source)
    if case=='post_hardlink':os.link(archive,archive.parent/'other.a')
    if case=='source_post':source.chmod(0o644);source.write_bytes(b'TEST_CODE_SOURCE_POST')
out=pathlib.Path(value('--out-dir'));out.mkdir(parents=True,exist_ok=True)
codegen=[args[i+1] for i,a in enumerate(args) if a=='-C'];suffix=next((a.split('=',1)[1] for a in codegen if a.startswith('extra-filename=')),'')
base=name+suffix;files=[out/base] if value('--crate-type')=='bin' else [out/('lib'+base+'.rmeta'),out/('lib'+base+'.rlib')]
for path in files:path.write_bytes(b'TEST_CODE_OUTPUT:'+name.encode()+b':'+source.read_bytes())
def esc(s):return s.replace(chr(92),chr(92)*2).replace(' ',chr(92)+' ').replace('#',chr(92)+'#').replace(':',chr(92)+':').replace('$','$$')
consumed=str(pathlib.Path(os.environ['OUT_DIR'])/'libpsm_s.a') if name=='psm' and case=='archive_consumed' else str(source)
(out/(base+'.d')).write_text(esc(str(files[0]))+': '+esc(consumed)+'\n')
if name=='psm' and case=='compiler_fail':sys.exit(7)
'''

PSM11_CARGO = r'''
import json,os,pathlib,shutil,subprocess,sys
CASE=__CASE__;TEMPLATE=__TEMPLATE__
args=sys.argv[1:]
def value(k):return args[args.index(k)+1]
app=pathlib.Path(value('--manifest-path')).parent;session=app.parent;target=pathlib.Path(value('--target-dir'))
root=session/'vendor/psm';host=target/'debug/deps';deps=target/'x86_64-apple-darwin/debug/deps'
for p in (host,deps):p.mkdir(parents=True,exist_ok=True)
package='registry+https://github.com/rust-lang/crates.io-index#psm@0.1.30'
out=target/'x86_64-apple-darwin/debug/build/psm-b26dbedb25ce9602/out';out.mkdir(parents=True)
loader=str(host)+':'+os.environ['DYLD_FALLBACK_LIBRARY_PATH']
def emit(e):print(json.dumps(e),flush=True)
def artifact(pkg,source,name,kind,files):
    return {'reason':'compiler-artifact','package_id':pkg,'manifest_path':str(root/'Cargo.toml') if pkg==package else str(source.parent.parent/'Cargo.toml'),
        'target':{'src_path':str(source),'kind':[kind],'crate_types':['bin' if kind=='custom-build' else kind],'name':name,'edition':'2021'},
        'features':[],'filenames':[str(p) for p in files],'executable':None,'fresh':False}
def compile(name,source,pkg,dest,kind='lib',extra=()):
    command=[os.environ['RUSTC_WRAPPER'],os.environ['RUSTC'],'--crate-name',name,'--edition=2021',str(source),'--crate-type',kind,'--emit='+('dep-info,link' if kind=='bin' else 'dep-info,metadata,link'),'--out-dir',str(dest),*extra]
    env=dict(os.environ,CARGO_MANIFEST_DIR=str(source.parent if source.name=='build.rs' else source.parent.parent),DYLD_FALLBACK_LIBRARY_PATH=loader,FIXTURE_HIT_ROOT=str(session/'compiler-entry'))
    if pkg==package:env.update(CARGO_PKG_NAME='psm',CARGO_PKG_VERSION='0.1.30')
    result=subprocess.run(command,env=env,cwd=pathlib.Path(env['CARGO_MANIFEST_DIR']))
    if result.returncode:raise RuntimeError('TEST_CODE prerequisite compile failed')
    files=[dest/name] if kind=='bin' else [dest/('lib'+name+'.rmeta'),dest/('lib'+name+'.rlib')]
    event=artifact(pkg,source,name,'custom-build' if kind=='bin' else kind,files)
    if kind=='bin':
        alias=dest/'build-script-build';shutil.copyfile(files[0],alias);event['filenames']=[str(alias)]
    return event
for name,version in (('cc','1.2.59'),('ar_archive_writer','0.5.1')):
    pkg='registry+https://github.com/rust-lang/crates.io-index#'+name+'@'+version
    if CASE!='missing_'+name:
        extra=['--target','x86_64-apple-darwin'] if CASE=='target_helper_'+name else ()
        emit(compile(name,session/'vendor'/name/'src/lib.rs',pkg,host,extra=extra))
    if CASE=='duplicate_'+name:emit(compile(name,session/'vendor'/name/'src/lib.rs',pkg,host))
externs=['--extern','cc='+str(host/'libcc.rlib'),'--extern','ar_archive_writer='+str(host/'libar_archive_writer.rlib')]
builder=compile('build_script_build',root/'build.rs',package,target/'debug/build/psm-89fcb6af8728a6c7','bin',externs)
if CASE=='builder_features':builder['features']=['other']
if CASE!='missing_builder':emit(builder)
if CASE=='duplicate_builder':emit(builder)
archive=out/'libpsm_s.a';archive.write_bytes(b'TEST_CODE_PSM_ARCHIVE')
event={'reason':'build-script-executed','package_id':package,'out_dir':str(out),'linked_libs':['static=psm_s'],'linked_paths':['native='+str(out)],'cfgs':['asm','link_asm','switchable_stack'],'env':[]}
if CASE=='event_cfg':event['cfgs'].reverse()
if CASE=='event_env':event['env']=[['OTHER','1']]
if CASE=='event_outdir':event['out_dir']=str(target)
if CASE!='missing_event':emit(event)
if CASE=='duplicate_event':emit(event)
argv=[os.environ['RUSTC']]+[v.format(source=root/'src/lib.rs',deps=deps,host=host,out=out) for v in TEMPLATE]
env=dict(os.environ,CARGO_MANIFEST_DIR=str(root),CARGO_MANIFEST_PATH=str(root/'Cargo.toml'),CARGO_PKG_NAME='psm',CARGO_PKG_VERSION='0.1.30',CARGO_PKG_VERSION_MAJOR='0',CARGO_PKG_VERSION_MINOR='1',CARGO_PKG_VERSION_PATCH='30',CARGO_PKG_VERSION_PRE='',CARGO_CRATE_NAME='psm',OUT_DIR=str(out),DYLD_FALLBACK_LIBRARY_PATH=loader,FIXTURE_HIT_ROOT=str(session/'compiler-entry'),PSM11_CASE=CASE)
cwd=root
if CASE=='no_native':del argv[-12:-8]
if CASE=='no_suffix':del argv[-12:]
if CASE in ('source_alias_no_native','manifest_alias_no_native'):
    del argv[-12:]
    for key in ('CARGO_PKG_NAME','CARGO_CRATE_NAME'):env.pop(key,None)
    argv[4]=str(root/'../psm/src/lib.rs') if CASE=='source_alias_no_native' else '../psm/src/lib.rs'
    env['CARGO_MANIFEST_DIR']=str(root.parent)+'//psm'
if CASE=='inline':argv[-10:-8]=['-lstatic=psm_s']
if CASE=='reorder':argv[-7],argv[-5]=argv[-5],argv[-7]
if CASE=='extra_native':argv[-12:-12]=['-L','native='+str(out)]
if CASE=='extra_cfg':argv[-12:-12]=['--cfg','other']
if CASE=='check_cfg':argv[19]='cfg(feature,values())'
if CASE=='link_arg':argv[-12:-12]=['-C','link-arg=-lother']
if CASE=='extern':argv[-12:-12]=['--extern','cc='+str(host/'libcc.rlib')]
if CASE=='extern_native':argv[-12:-12]=['--extern-native','foreign']
if CASE=='test':argv[-12:-12]=['--test']
if CASE=='host':i=argv.index('--target');del argv[i:i+2]
if CASE=='platform':argv[argv.index('--target')+1]='aarch64-apple-darwin'
if CASE=='source':argv[4]=str(app/'src/lib.rs')
if CASE=='manifest_alias':env['CARGO_MANIFEST_DIR']=str(root.parent)+'//'+name
if CASE=='package':env['CARGO_PKG_NAME']='other'
if CASE=='version':env['CARGO_PKG_VERSION']='0.1.31'
if CASE=='cwd':cwd=app
if CASE=='outdir':env['OUT_DIR']=str(target)
if CASE=='feature':env['CARGO_FEATURE_OTHER']='1'
if CASE=='environment':env['AR']='/TEST_CODE_FOREIGN_AR'
if CASE=='wrapper':env['RUSTC_WRAPPER']=str(session/'other-wrapper')
original=(root/'src/lib.rs').read_bytes()
if CASE=='source_hash':(root/'src/lib.rs').chmod(0o644);(root/'src/lib.rs').write_bytes(b'TEST_CODE_DRIFT')
if CASE=='pre_missing':archive.unlink()
if CASE=='pre_symlink':archive.unlink();archive.symlink_to(root/'src/lib.rs')
if CASE=='pre_hardlink':os.link(archive,out/'other.a')
(session/'psm11-attempt.json').write_text(json.dumps({'argv_hex':[os.fsencode(a).hex() for a in argv]}))
result=subprocess.run([os.environ['RUSTC_WRAPPER'],*argv],env=env,cwd=cwd)
if CASE in ('source_hash','source_post'):(root/'src/lib.rs').write_bytes(original)
if result.returncode:emit({'reason':'build-finished','success':False});sys.exit(result.returncode)
consumer=artifact(package,root/'src/lib.rs','psm','lib',[deps/'libpsm-749f77748b047fe7.rmeta',deps/'libpsm-749f77748b047fe7.rlib'])
if CASE=='consumer_features':consumer['features']=['other']
if CASE=='consumer_role':consumer['target']['kind']=['proc-macro']
if CASE!='missing_consumer':emit(consumer)
if CASE=='duplicate_consumer':emit(consumer)
app_event=compile('stock_analysis',app/'src/lib.rs','TEST_CODE_app',deps,extra=['--target','x86_64-apple-darwin'])
if CASE=='archive_selected':app_event['filenames']=[str(archive)]
if CASE in ('archive_copy','archive_retained_copy','archive_current_output','archive_retained_extern','archive_retained_consumed'):
    copied=deps/'libpromoted.rlib';shutil.copyfile(archive,copied)
    if CASE in ('archive_copy','archive_retained_copy'):app_event['filenames']=[str(copied)]
emit(app_event)
paths=list((session/'invocations').glob('*/receipt.json'));rpath=next(p for p in paths if json.loads(p.read_text()).get('context',{}).get('psm_static_declaration'))
r=json.loads(rpath.read_text());bpath=next(p for p in paths if json.loads(p.read_text()).get('source')==str(root/'build.rs'))
if CASE=='helper_cwd_metadata':
    p=next(p for p in paths if json.loads(p.read_text()).get('source')==str(session/'vendor/cc/src/lib.rs'))
    for leaf in ('invocation.json','receipt.json'):
        target=p.parent/leaf;data=json.loads(target.read_text());data['cwd']=str(app);target.write_text(json.dumps(data))
if CASE.startswith('target_helper_'):
    name=CASE.removeprefix('target_helper_')
    p=next(p for p in paths if json.loads(p.read_text()).get('source')==str(session/'vendor'/name/'src/lib.rs'))
    for leaf in ('invocation.json','receipt.json'):
        target=p.parent/leaf;data=json.loads(target.read_text());data['role']='Host';target.write_text(json.dumps(data))
if CASE in ('archive_retained_copy','archive_retained_extern','archive_retained_consumed'):
    archive.write_bytes(b'TEST_CODE_FINAL_CHANGE')
    for phase in ('pre','post'):(rpath.parent/r['psm_archive_'+phase]['snapshot']).unlink()
if CASE in ('archive_current_output','archive_retained_extern','archive_retained_consumed'):
    p=next(p for p in paths if json.loads(p.read_text()).get('source')==str(app/'src/lib.rs'));data=json.loads(p.read_text())
    if CASE=='archive_current_output':
        ordinary=next(o for o in data['outputs'] if o['kind']=='link');path=pathlib.Path(ordinary['path']);shutil.copyfile(copied,path)
    if CASE=='archive_retained_extern':data['externs'].append({'name':'psm','path':str(copied)})
    if CASE=='archive_retained_consumed':
        next(o for o in data['outputs'] if o['kind']=='dep-info')['dep_info']['paths'].append(str(copied))
    p.write_text(json.dumps(data))
if CASE=='request_only':
    for leaf in ('invocation.json','receipt.json'):(rpath.parent/leaf).unlink()
if CASE=='missing_annotation':
    for leaf in ('invocation.json','receipt.json'):
        p=rpath.parent/leaf;data=json.loads(p.read_text());data['context']={'kind':'DirectCargoCompile'}
        for key in tuple(data):
            if key.startswith('psm_archive_'):del data[key]
        p.write_text(json.dumps(data))
if CASE=='raw_builder':
    p=bpath.parent/'request.json';data=json.loads(p.read_text());data['argv_hex'][4]=os.fsencode(str(app/'build.rs')).hex();p.write_text(json.dumps(data))
if CASE=='request_cwd':
    p=rpath.parent/'request.json';data=json.loads(p.read_text());data['cwd_hex']=os.fsencode(str(app)).hex();p.write_text(json.dumps(data))
if CASE=='snapshot_missing':(rpath.parent/r['psm_archive_pre']['snapshot']).unlink()
if CASE=='snapshot_changed':(rpath.parent/r['psm_archive_post']['snapshot']).write_bytes(b'TEST_CODE_TAMPER')
if CASE=='final_archive':archive.write_bytes(b'TEST_CODE_FINAL_CHANGE')
if CASE in ('archive_output','archive_extern','snapshot_output'):
    p=next(p for p in paths if json.loads(p.read_text()).get('source')==str(app/'src/lib.rs'));data=json.loads(p.read_text())
    if CASE=='archive_extern':data['externs'].append({'name':'psm','path':str(archive)})
    else:data['declared_outputs'].append({'path':str(rpath.parent/r['psm_archive_pre']['snapshot']) if CASE=='snapshot_output' else str(archive),'kind':'link'})
    p.write_text(json.dumps(data))
emit({'reason':'build-finished','success':True})
'''

# Original ordered record12 words, with only session paths substituted.
ZSTD12_ARGS = ['--crate-name', 'zstd_sys', '--edition=2018', '{source}', '--error-format=json', '--json=diagnostic-rendered-ansi,artifacts,future-incompat', '--crate-type', 'lib', '--emit=dep-info,metadata,link', '-C', 'embed-bitcode=no', '-C', 'debuginfo=1', '-C', 'split-debuginfo=unpacked', '--allow=non_upper_case_globals', '--cfg', 'feature="legacy"', '--cfg', 'feature="std"', '--cfg', 'feature="zdict_builder"', '--check-cfg', 'cfg(docsrs,test)', '--check-cfg', 'cfg(feature, values("bindgen", "debug", "default", "experimental", "fat-lto", "legacy", "no_asm", "no_wasm_shim", "non-cargo", "pkg-config", "seekable", "std", "thin", "thin-lto", "zdict_builder", "zstdmt"))', '-C', 'metadata=219fc208899e9805', '-C', 'extra-filename=-d231fa1e57295f5a', '--out-dir', '{deps}', '--target', 'x86_64-apple-darwin', '-L', 'dependency={deps}', '-L', 'dependency={host}', '--cap-lints', 'allow', '-L', 'native={out}', '-l', 'static=zstd']
SERDE12_CHECK = 'cfg(feature, values("alloc", "default", "rc", "result", "std", "unstable"))'
SERDE12_CHECKS = ['if_docsrs_then_no_serde_core', 'no_core_cstr', 'no_core_error', 'no_core_net', 'no_core_num_saturating', 'no_diagnostic_namespace', 'no_serde_derive', 'no_std_atomic', 'no_std_atomic64', 'no_target_has_atomic']
TOOLS12_PRIVATE = b'#[doc(hidden)]\npub mod __private228 {\n    #[doc(hidden)]\n    pub use crate::private::*;\n}\n'

TOOLS12_RUSTC = r'''
import json,os,pathlib,sys
args=sys.argv[1:]
def value(k):
    inline=[a.split('=',1)[1] for a in args if a.startswith(k+'=')]
    return inline[0] if inline else args[args.index(k)+1]
name=value('--crate-name');source=next(pathlib.Path(a) for a in args if a.endswith('.rs'))
source=source if source.is_absolute() else pathlib.Path.cwd()/source
probe='--cfg=anyhow_build_probe' in args
hits=pathlib.Path(os.environ['FIXTURE_HIT_ROOT']);hits.mkdir(parents=True,exist_ok=True)
(hits/(('probe-' if probe else 'compile-')+name)).write_text(json.dumps(args))
case=os.environ.get('TOOLS12_CASE','normal');family=os.environ.get('TOOLS12_FAMILY','')
if name=='zstd_sys':
    archive=pathlib.Path(os.environ['OUT_DIR'])/'libzstd.a'
    if case=='during_change':archive.write_bytes(b'TEST_CODE_CHANGED_DURING_CHILD')
    if case=='post_missing':archive.unlink()
    if case=='post_symlink':archive.unlink();archive.symlink_to(source)
    if case=='post_hardlink':os.link(archive,archive.parent/'other.a')
if (name=='zstd_sys' or probe) and case=='source_post':source.chmod(0o644);source.write_bytes(b'TEST_CODE_SOURCE_POST')
out=pathlib.Path(value('--out-dir'));out.mkdir(parents=True,exist_ok=True)
codegen=[args[i+1] for i,a in enumerate(args) if a=='-C'];suffix=next((v.split('=',1)[1] for v in codegen if v.startswith('extra-filename=')),'')
base=name+suffix;types=value('--crate-type');emit=value('--emit').split(',')
files=[]
if 'metadata' in emit:files.append(out/('lib'+base+'.rmeta'))
if 'link' in emit:files.append(out/(base if types=='bin' else 'lib'+base+'.rlib'))
status=1 if probe and case in ('probe1-none','probe1-partial','probe1-both') else (7 if (probe and case=='probe7') or (name=='zstd_sys' and case=='compiler_fail') else 0)
for path in files:
    if not (probe and (case in ('probe1-none','probe1-partial') or case=='capture_missing')):
        path.write_bytes(b'TEST_CODE_OUTPUT:'+name.encode()+b':'+json.dumps(args).encode())
def esc(s):return s.replace(chr(92),chr(92)*2).replace(' ',chr(92)+' ').replace('#',chr(92)+'#').replace(':',chr(92)+':').replace('$','$$')
consumed=[str(source)]
if name=='serde_core':consumed.extend([str(source.parent/'crate_root.rs'),str(pathlib.Path(os.environ['OUT_DIR'])/'private.rs')])
dep=out/(base+'.d')
if not(probe and case=='probe1-none'):
    dep.write_text(esc(str(files[0] if files else out/base))+': '+' '.join(esc(s) for s in consumed)+'\n'+('# env-dep:OUT_DIR='+os.environ['OUT_DIR']+'\n' if name=='serde_core' else ''))
if probe and case=='capture_alias':
    path=out/'libanyhow.rmeta';path.unlink();path.symlink_to(source)
if probe and case=='capture_hardlink':os.link(out/'libanyhow.rmeta',out/'other.rmeta')
if probe and case=='capture_dep':dep.write_text('# TEST_CODE invalid dep comment\n')
sys.exit(status)
'''

TOOLS12_CARGO = r'''
import json,os,pathlib,shutil,subprocess,sys
CASE=__CASE__;FAMILY=__FAMILY__;ZSTD=__ZSTD__;SCHECK=__SCHECK__;CHECKS=__CHECKS__;PRIVATE=__PRIVATE__
args=sys.argv[1:]
def value(k):return args[args.index(k)+1]
app=pathlib.Path(value('--manifest-path')).parent;session=app.parent;target=pathlib.Path(value('--target-dir'))
name={'zstd':'zstd-sys','anyhow':'anyhow','serde':'serde_core'}[FAMILY]
version={'zstd':'2.0.16+zstd.1.5.7','anyhow':'1.0.102','serde':'1.0.228'}[FAMILY]
package='registry+https://github.com/rust-lang/crates.io-index#'+name+'@'+version;root=session/'vendor'/name
host=target/'debug/deps';deps=target/'x86_64-apple-darwin/debug/deps'
for p in (host,deps):p.mkdir(parents=True,exist_ok=True)
loader=str(host)+':'+os.environ['DYLD_FALLBACK_LIBRARY_PATH'];features={'zstd':['legacy','std','zdict_builder'],'anyhow':['default','std'],'serde':['alloc','default','rc','result','std']}[FAMILY]
out=(target/'debug/build/anyhow-d005d5c1d4426d3a/out' if FAMILY=='anyhow' else target/'x86_64-apple-darwin/debug/build'/({'zstd':'zstd-sys-21005f2c27aa00ab','serde':'serde_core-8c92ebf254a84c42'}[FAMILY])/'out');out.mkdir(parents=True)
def emit(e):print(json.dumps(e),flush=True)
def environment(pkgname,pkgversion,manifest,crate,*,nested=False):
    parts=pkgversion.split('+')[0].split('.')
    return dict(os.environ,CARGO_MANIFEST_DIR=str(manifest),CARGO_MANIFEST_PATH=str(manifest/'Cargo.toml'),CARGO_PKG_NAME=pkgname,CARGO_PKG_VERSION=pkgversion,
        CARGO_PKG_VERSION_MAJOR=parts[0],CARGO_PKG_VERSION_MINOR=parts[1],CARGO_PKG_VERSION_PATCH=parts[2],CARGO_PKG_VERSION_PRE='',CARGO_CRATE_NAME=crate,
        DYLD_FALLBACK_LIBRARY_PATH=loader,FIXTURE_HIT_ROOT=str(session/'compiler-entry'),TOOLS12_CASE=CASE,TOOLS12_FAMILY=FAMILY)
def artifact(pkg,source,crate,kind,files,fs):
    return {'reason':'compiler-artifact','package_id':pkg,'manifest_path':str((source.parent if source.name=='build.rs' else source.parent.parent)/'Cargo.toml'),
        'target':{'src_path':str(source),'kind':[kind],'crate_types':['bin' if kind=='custom-build' else kind],'name':'build-script-build' if kind=='custom-build' else crate,'edition':'2018' if FAMILY=='zstd' and pkg!= 'TEST_CODE_app' else '2021'},
        'features':fs,'filenames':[str(p) for p in files],'executable':None,'fresh':False}
def compile_event(crate,source,pkg,dest,fs=(),kind='lib',extra=(),exact=None,env_extra=None):
    manifest=source.parent if source.name=='build.rs' else source.parent.parent
    pkgname,pkgversion=(name,version) if pkg==package else (manifest.name,{'cc':'1.2.59','pkg-config':'0.3.32'}.get(manifest.name,'0.0.0'))
    argv=exact or [os.environ['RUSTC'],'--crate-name',crate,'--edition='+('2018' if FAMILY=='zstd' and pkg!='TEST_CODE_app' else '2021'),str(source),'--crate-type',kind,'--emit='+('dep-info,link' if kind=='bin' else 'dep-info,metadata,link'),'--out-dir',str(dest),*[v for f in fs for v in ('--cfg','feature="'+f+'"')],*extra]
    env=environment(pkgname,pkgversion,manifest,crate);env.update(env_extra or {})
    r=subprocess.run([os.environ['RUSTC_WRAPPER'],*argv],env=env,cwd=manifest)
    if r.returncode:raise RuntimeError('TEST_CODE prerequisite compile failed '+crate)
    c=[argv[i+1] for i,a in enumerate(argv) if a=='-C'];suffix=next((v.split('=',1)[1] for v in c if v.startswith('extra-filename=')),'')
    base=crate+suffix;files=[dest/base] if kind=='bin' else [dest/('lib'+base+'.rmeta'),dest/('lib'+base+'.rlib')]
    event=artifact(pkg,source,crate,'custom-build' if kind=='bin' else kind,files,list(fs))
    if kind=='bin':
        alias=dest/'build-script-build';shutil.copyfile(files[0],alias);event['filenames']=[str(alias)]
    return event
if FAMILY=='zstd':
    for helper,hversion,crate,hfeatures in (('cc','1.2.59','cc',['parallel']),('pkg-config','0.3.32','pkg_config',[])):
        hp='registry+https://github.com/rust-lang/crates.io-index#'+helper+'@'+hversion
        event=compile_event(crate,session/'vendor'/helper/'src/lib.rs',hp,host,hfeatures,extra=['--target','x86_64-apple-darwin'] if CASE=='target_helper_'+crate else ())
        if CASE!='missing_'+crate:emit(event)
        if CASE=='duplicate_'+crate:emit(event)
    extern=['--extern','cc='+str(host/'libcc.rlib'),'--extern','pkg_config='+str(host/'libpkg_config.rlib')]
else:extern=[]
def serde_args(fs,metadata,suffix,builder):
    dest=target/'debug/build'/('serde_core'+suffix) if builder else deps
    argv=[os.environ['RUSTC'],'--crate-name','build_script_build' if builder else 'serde_core','--edition=2021',str(root/('build.rs' if builder else 'src/lib.rs')),
        '--error-format=json','--json=diagnostic-rendered-ansi,artifacts,future-incompat','--crate-type','bin' if builder else 'lib','--emit='+('dep-info,link' if builder else 'dep-info,metadata,link'),
        '-C','embed-bitcode=no','-C','debuginfo=1','-C','split-debuginfo=unpacked',*[v for f in fs for v in ('--cfg','feature="'+f+'"')],
        '--check-cfg','cfg(docsrs,test)','--check-cfg',SCHECK,'-C','metadata='+metadata,'-C','extra-filename='+suffix,'--out-dir',str(dest)]
    if not builder:argv+=['--target','x86_64-apple-darwin','-L','dependency='+str(deps)]
    argv+=['-L','dependency='+str(host),'--cap-lints','allow']
    if not builder:argv += [v for f in CHECKS for v in ('--check-cfg','cfg('+f+')')]
    return argv,dest
if FAMILY=='serde':
    for tag,fs,metadata,suffix in [('A',features if CASE!='no_compatible' else ['result','std'],'499268712182025e','-7695a1447424a441'),('B',['result','std'],'1f52e4bdee6a9d81','-44ac0d1892dbd87b')]+([('C',features,'aaaaaaaaaaaaaaaa','-aaaaaaaaaaaaaaaa')] if CASE in ('duplicate_features','private_hash_same') else []):
        argv,dest=serde_args(fs,metadata,suffix,True)
        e=compile_event('build_script_build',root/'build.rs',package,dest,fs,'bin',exact=argv)
        if CASE=='builder_features' and tag=='A':e['features']=['result','std']
        if not(CASE=='missing_builder' and tag=='A'):emit(e)
    (out/'private.rs').write_bytes(PRIVATE if CASE!='private_bytes' else b'TEST_CODE_WRONG_PRIVATE')
else:
    builder=compile_event('build_script_build',root/'build.rs',package,target/'debug/build'/('zstd-sys-0c68b4e77f2808a6a' if FAMILY=='zstd' else 'anyhow-d005d5c1d4426d3a'),features,'bin',extern)
    if CASE=='builder_features':builder['features']=['other']
    if CASE!='missing_builder':emit(builder)
    if CASE=='duplicate_builder':emit(builder)
    if FAMILY=='zstd':(out/'libzstd.a').write_bytes(b'TEST_CODE_ZSTD_ARCHIVE')
child=None;original=(root/('src/nightly.rs' if FAMILY=='anyhow' else 'src/lib.rs')).read_bytes()
if FAMILY=='zstd':
    argv=[os.environ['RUSTC']]+[v.format(source=root/'src/lib.rs',deps=deps,host=host,out=out) for v in ZSTD]
    env=environment(name,version,root,'zstd_sys');env['OUT_DIR']=str(out);cwd=root
    if CASE=='source':argv[4]=str(app/'src/lib.rs')
    if CASE=='raw_space':
        feature_checks=[word for word in argv if word.startswith('cfg(feature, values(')]
        assert len(feature_checks)==1, 'TEST_CODE raw_space requires one feature check word'
        original_check=feature_checks[0];mutated_check=original_check.replace(', ', ',')
        assert mutated_check!=original_check, 'TEST_CODE raw_space must change feature check bytes'
        argv[argv.index(original_check)]=mutated_check
    if CASE=='raw_feature':argv[argv.index('feature="legacy"')]='feature="experimental"'
    if CASE=='source_alias':argv[4]=str(root/'../zstd-sys/src/lib.rs')
    if CASE=='inline':argv[-2:]=['-lstatic=zstd']
    if CASE=='reorder':argv[-4:]=argv[-2:]+argv[-4:-2]
    if CASE=='extra_native':argv[-4:-4]=['-L','native='+str(out)]
    if CASE=='no_native':del argv[-4:]
    if CASE=='extern':argv[-4:-4]=['--extern','cc='+str(host/'libcc.rlib')]
    if CASE=='link_arg':argv[-4:-4]=['-C','link-arg=-lother']
    if CASE=='host':i=argv.index('--target');del argv[i:i+2]
    if CASE=='platform':argv[argv.index('--target')+1]='aarch64-apple-darwin'
    archive=out/'libzstd.a'
    if CASE=='pre_missing':archive.unlink()
    if CASE=='pre_symlink':archive.unlink();archive.symlink_to(root/'src/lib.rs')
    if CASE=='pre_hardlink':os.link(archive,out/'other.a')
elif FAMILY=='anyhow':
    argv=[os.environ['RUSTC'],'--cfg=anyhow_build_probe','--edition=2018','--crate-name=anyhow','--crate-type=lib','--cap-lints=allow','--emit=dep-info,metadata','--out-dir',str(out/'probe'),'src/nightly.rs','--target','x86_64-apple-darwin']
    env=environment(name,version,root,'anyhow');env.update(OUT_DIR=str(out),HOST='x86_64-apple-darwin',TARGET='x86_64-apple-darwin',CARGO_FEATURE_DEFAULT='1',CARGO_FEATURE_STD='1',CARGO_ENCODED_RUSTFLAGS='',DYLD_FALLBACK_LIBRARY_PATH=str(target/'debug')+':'+str(host)+':'+str(pathlib.Path(os.environ['DYLD_FALLBACK_LIBRARY_PATH'])/'rustlib/x86_64-apple-darwin/lib')+':'+os.environ['DYLD_FALLBACK_LIBRARY_PATH']);cwd=root
    if CASE=='source':argv[9]='src/lib.rs'
    if CASE in ('probe_check_cfg','probe_check_cfg_source'):argv[1:2]=['--check-cfg','cfg(anyhow_build_probe)']
    if CASE=='probe_check_cfg_source':argv[argv.index('src/nightly.rs')]='src/lib.rs'
    if CASE=='inline':argv[1:2]=['--cfg','anyhow_build_probe']
    if CASE=='raw_space':argv[6]='--emit=metadata,dep-info'
    if CASE=='source_alias':argv[9]='./src/nightly.rs'
    if CASE=='host_env':env['HOST']='aarch64-apple-darwin'
    if CASE=='encoded_flags':env['CARGO_ENCODED_RUSTFLAGS']='-Cdebuginfo=2'
    if CASE=='missing_feature':env.pop('CARGO_FEATURE_STD')
    if CASE=='platform':argv[-1]='aarch64-apple-darwin'
    if CASE=='host':del argv[-2:]
    if CASE=='loader':env['DYLD_FALLBACK_LIBRARY_PATH']=loader
    if CASE=='retry':argv.insert(1,'--cfg=anyhow_build_probe')
else:
    argv,_=serde_args(features,'accbf619672d37e1','-d3ba454884ccf462',False)
    env=environment(name,version,root,'serde_core');env['OUT_DIR']=str(out);cwd=root
    if CASE=='consumer_role':i=argv.index('--target');del argv[i:i+2]
    if CASE=='consumer_metadata':argv[argv.index('metadata=accbf619672d37e1')]='metadata=other'
if CASE=='manifest_alias':env['CARGO_MANIFEST_DIR']=str(root.parent)+'//'+name
if CASE=='package':env['CARGO_PKG_NAME']='other'
if CASE=='version':env['CARGO_PKG_VERSION']='9.9.9'
if CASE=='cwd':cwd=app
if CASE=='outdir':env['OUT_DIR']=str(target)
if CASE=='feature':env['CARGO_FEATURE_OTHER']='1'
if CASE=='bootstrap':env['RUSTC_BOOTSTRAP']='1'
if CASE=='stage':env['RUSTC_STAGE']='1'
if CASE=='wrapper':env['RUSTC_WRAPPER']=str(session/'other-wrapper')
if CASE=='source_hash':
    source=root/('src/nightly.rs' if FAMILY=='anyhow' else 'src/lib.rs');source.chmod(0o644);source.write_bytes(b'TEST_CODE_DRIFT')
(session/'tools12-attempt.json').write_text(json.dumps({'argv_hex':[os.fsencode(a).hex() for a in argv]}))
result=subprocess.run([os.environ['RUSTC_WRAPPER'],*argv],env=env,cwd=cwd)
if CASE in ('source_hash','source_post'):(root/('src/nightly.rs' if FAMILY=='anyhow' else 'src/lib.rs')).write_bytes(original)
# Simulate the observed build-script cleanup after the wrapper retained outputs.
if FAMILY=='anyhow':shutil.rmtree(out/'probe',ignore_errors=True)
if FAMILY=='zstd' and result.returncode:
    emit({'reason':'build-finished','success':False});sys.exit(result.returncode)
if FAMILY=='anyhow':
    status=1 if CASE.startswith('probe1') else (7 if CASE=='probe7' else 0)
    cfgs=['error_generic_member_access'] if status==0 else []
    checks=('anyhow_build_probe','anyhow_nightly_testing','anyhow_no_clippy_format_args','anyhow_no_core_error','error_generic_member_access')
    consumer=compile_event('anyhow',root/'src/lib.rs',package,host,features,extra=[v for c in cfgs for v in ('--cfg',c)]+[v for c in checks for v in ('--check-cfg','cfg('+c+')')],env_extra={'OUT_DIR':str(out)})
else:
    cfgs=[];crate='zstd_sys' if FAMILY=='zstd' else 'serde_core';suffix='-d231fa1e57295f5a' if FAMILY=='zstd' else '-d3ba454884ccf462'
    consumer=artifact(package,root/'src/lib.rs',crate,'lib',[deps/('lib'+crate+suffix+'.rmeta'),deps/('lib'+crate+suffix+'.rlib')],features)
if CASE=='consumer_features':consumer['features']=['other']
if CASE!='missing_consumer':emit(consumer)
if CASE=='duplicate_consumer':emit(consumer)
event={'reason':'build-script-executed','package_id':package,'out_dir':str(out),'linked_libs':['static=zstd'] if FAMILY=='zstd' else [],'linked_paths':['native='+str(out)] if FAMILY=='zstd' else [],'cfgs':cfgs,'env':[]}
if CASE=='event_cfg':event['cfgs']=['other']
if CASE=='event_env':event['env']=[['OTHER','1']]
if CASE=='event_outdir':event['out_dir']=str(target)
if CASE!='missing_event':emit(event)
if CASE=='duplicate_event':emit(event)
app_event=compile_event('stock_analysis',app/'src/lib.rs','TEST_CODE_app',deps,extra=['--target','x86_64-apple-darwin'])
paths=list((session/'invocations').glob('*/receipt.json'))
child_path=next((p for p in paths if json.loads(p.read_text()).get('context',{}).get('zstd_static_declaration') or json.loads(p.read_text()).get('context',{}).get('kind')=='AnyhowStaticFeatureProbe'),None)
consumer_path=next((p for p in paths if json.loads(p.read_text()).get('source')==str(root/'src/lib.rs')),None)
bpaths=[p for p in paths if json.loads(p.read_text()).get('source')==str(root/'build.rs')]
selected_path=next(p for p in paths if json.loads(p.read_text()).get('source')==str(app/'src/lib.rs'))
if FAMILY=='serde' and CASE in ('raw_builder','builder_source','builder_cwd','builder_metadata','builder_role','builder_outdir','alias'):
    consumer=json.loads(consumer_path.read_text());wide_cfg=consumer['parsed']['options'].get('--cfg',[])
    assert wide_cfg==['feature="'+f+'"' for f in features], 'TEST_CODE builder control requires exact consumer features'
    compatible=[p for p in bpaths if json.loads(p.read_text())['parsed']['options'].get('--cfg',[])==wide_cfg]
    assert len(bpaths)==2 and len(compatible)==1, 'TEST_CODE builder control requires two builders and one feature-compatible target'
    controlled_builder,=compatible;builder=json.loads(controlled_builder.read_text())
    request=json.loads((controlled_builder.parent/'request.json').read_text());initial=json.loads((controlled_builder.parent/'invocation.json').read_text())
    raw=[os.fsdecode(bytes.fromhex(a)) for a in request['argv_hex']]
    raw_cfg=[raw[i+1] for i,word in enumerate(raw) if word=='--cfg']
    raw_sources=[word for word in raw[1:] if not word.startswith('-') and word.endswith('.rs')]
    assert request['argv_hex']==initial['argv_hex']==builder['argv_hex'] and raw_cfg==wide_cfg, 'TEST_CODE compatible target raw/receipt feature mismatch'
    assert raw[0]==os.environ['RUSTC'] and raw_sources==[str(root/'build.rs')], 'TEST_CODE compatible target compiler/source mismatch'
    assert builder['source']==str(root/'build.rs') and builder['package']['id']==package and builder['role']=='Host' and builder['kind']=='Compile', 'TEST_CODE compatible target identity mismatch'
    assert builder['context']==initial['context']=={'kind':'DirectCargoCompile'} and builder['compiler_sha256']==initial['compiler_sha256']==consumer['compiler_sha256'], 'TEST_CODE compatible target compiler context mismatch'
    assert request['cwd_hex']==os.fsencode(str(root)).hex() and builder['cwd']==initial['cwd']==str(root), 'TEST_CODE compatible target cwd mismatch'
def update(path,change,leaves=('receipt.json',)):
    for leaf in leaves:
        p=path.parent/leaf;data=json.loads(p.read_text());change(data);p.write_text(json.dumps(data))
if CASE=='request_only' and child_path:
    for leaf in ('receipt.json','invocation.json'):(child_path.parent/leaf).unlink()
if CASE=='missing_annotation' and child_path:
    def erase(d):
        d['context']={'kind':'DirectCargoCompile'}
        for k in list(d):
            if k.startswith('zstd_archive_'):del d[k]
    update(child_path,erase,('receipt.json','invocation.json'))
if CASE=='request_cwd' and child_path:update(child_path,lambda d:d.update(cwd_hex=os.fsencode(str(app)).hex()),('request.json',))
if CASE=='raw_builder':update(controlled_builder if FAMILY=='serde' else bpaths[0],lambda d:d['argv_hex'].__setitem__(4,os.fsencode(str(app/'build.rs')).hex()),('request.json',))
if CASE=='consumer_source':update(consumer_path,lambda d:d.update(source=str(app/'src/lib.rs')),('receipt.json','invocation.json'))
if CASE=='builder_source':update(controlled_builder,lambda d:d.update(source=str(app/'build.rs')),('receipt.json','invocation.json'))
if CASE=='consumer_outdir':update(consumer_path,lambda d:d['environment_hex'].update({os.fsencode('OUT_DIR').hex():os.fsencode(str(target)).hex()}),('request.json','receipt.json','invocation.json'))
if CASE=='compiler_identity':update(consumer_path,lambda d:d.update(compiler_sha256='0'*64),('receipt.json','invocation.json'))
if CASE=='builder_cwd':update(controlled_builder,lambda d:d.update(cwd=str(app)),('receipt.json','invocation.json'))
if CASE=='builder_metadata':update(controlled_builder,lambda d:d['parsed']['codegen'].update(metadata=['other']),('receipt.json','invocation.json'))
if CASE=='builder_role':update(controlled_builder,lambda d:d.update(role='Target'),('receipt.json','invocation.json'))
if CASE=='builder_outdir':update(controlled_builder,lambda d:d['environment_hex'].update({os.fsencode('OUT_DIR').hex():os.fsencode(str(out)).hex()}),('request.json','receipt.json','invocation.json'))
if CASE=='alias':
    links=[o['path'] for o in json.loads(controlled_builder.read_text())['declared_outputs'] if o['kind']=='link']
    assert len(links)==1, 'TEST_CODE compatible target requires one real compiler link output'
    pathlib.Path(links[0]).write_bytes(b'TEST_CODE_ALIAS_DRIFT')
if CASE=='snapshot_missing' and child_path:(child_path.parent/('zstd-archive-pre.raw' if FAMILY=='zstd' else 'probe-output-0.raw')).unlink()
if CASE=='snapshot_changed' and child_path:(child_path.parent/('zstd-archive-post.raw' if FAMILY=='zstd' else 'probe-output-1.raw')).write_bytes(b'TEST_CODE_TAMPER')
if CASE=='final_archive':(out/'libzstd.a').write_bytes(b'TEST_CODE_FINAL_CHANGE')
if CASE.startswith('namespace_'):
    _,epoch,reuse=CASE.split('_',2)
    child=json.loads(child_path.read_text())
    source=out/'libzstd.a' if FAMILY=='zstd' else child_path.parent/'probe-output-1.raw'
    copied=deps/'libpromoted.rlib';shutil.copyfile(source,copied)
    if epoch=='retained':
        if FAMILY=='zstd':
            (out/'libzstd.a').write_bytes(b'TEST_CODE_FINAL_CHANGE')
            for phase in ('pre','post'):(child_path.parent/('zstd-archive-'+phase+'.raw')).unlink()
        else:
            for o in child['outputs']:(child_path.parent/o['snapshot']).unlink()
    if reuse=='snapshot':update(selected_path,lambda d:d['declared_outputs'].append({'path':str(child_path.parent/('zstd-archive-pre.raw' if FAMILY=='zstd' else 'probe-output-0.raw')),'kind':'link'}))
    if reuse in ('artifact','selected'):app_event['filenames']=[str(copied)]
    if reuse=='output':
        data=json.loads(selected_path.read_text());ordinary=next(o for o in data['outputs'] if o['kind']=='link');shutil.copyfile(copied,ordinary['path'])
    if reuse=='extern':update(selected_path,lambda d:d['externs'].append({'name':'promoted','path':str(copied)}))
    if reuse=='consumed':update(selected_path,lambda d:next(o for o in d['outputs'] if o['kind']=='dep-info')['dep_info']['paths'].append(str(copied)))
emit(app_event);emit({'reason':'build-finished','success':True})
'''

# Stage A fixture: fresh synthetic tools; no real Cargo, compiler or archive.
E1_NATIVE = r'''
import json,os,pathlib,signal,sys,threading,uuid
args=sys.argv[1:];env=dict(os.environ);session=pathlib.Path(env['E1_SESSION']);case=env['E1_CASE']
hits=session/'foreign-entry';hits.mkdir(exist_ok=True)
pair=tuple(map(int,env['CARGO_MAKEFLAGS'].split('--jobserver-fds=')[1].split()[0].split(',')))
entry={'argv':args,'environment':env,'cwd':str(pathlib.Path.cwd()),'stdin_eof':sys.stdin.buffer.read()==b'',
       'fds':list(pair),'inodes':[os.fstat(fd).st_ino for fd in pair]}
(hits/uuid.uuid4().hex).write_text(json.dumps(entry))
if args[0]=='-E':
    source=pathlib.Path(args[-1])
    if case=='post_missing':source.unlink()
    if case=='post_change':source.write_bytes(b'X'*206)
    if case=='source_post':
        manifest=pathlib.Path(env['CARGO_MANIFEST_PATH']);manifest.chmod(0o644);manifest.write_bytes(b'TEST_CODE_drift')
    if case.startswith('warning_') and '--' not in args:
        os.write(1 if case=='warning_stdout' else 2,b'-Wslash-u-filename\n');sys.exit(3)
    if case=='streams':
        threads=[threading.Thread(target=os.write,args=(fd,byte*120000)) for fd,byte in ((1,b'O'),(2,b'E'))]
        for t in threads:t.start()
        for t in threads:t.join()
    else:os.write(1,b'"clang" "gcc"\n')
    if case=='signal':os.kill(os.getpid(),signal.SIGTERM)
    sys.exit(7 if case=='e_nonzero' else 0)
if args==['-?']:
    os.write(1,b'TEST_CODE_help_stdout\n');os.write(2,b'TEST_CODE_help_stderr\n')
    sys.exit(0 if case=='help_zero' else 9)
assert args==['--version']
os.write(1,b'ziglang TEST_CODE\n' if case in ('version_zig','version_nonzero') else b'Apple clang TEST_CODE\n')
os.write(2,b'TEST_CODE_version_stderr\n');sys.exit(4 if case=='version_nonzero' else 0)
'''

E1_CARGO = r'''
import hashlib,json,os,pathlib,subprocess,sys,threading
CASE=__CASE__;argv=sys.argv[1:];app=pathlib.Path(argv[argv.index('--manifest-path')+1]).parent;session=app.parent
target=session/'target';host=target/'debug/deps';deps=target/'x86_64-apple-darwin/debug/deps'
base=dict(os.environ,E1_SESSION=str(session),E1_CASE=CASE,D1_SESSION=str(session),D1_CASE='normal',CARGO_ENCODED_RUSTFLAGS='')
def emit(e):print(json.dumps(e),flush=True)
def compile(name,source,pkg,dest,extra=()):
    root=source.parent.parent;env=dict(base,CARGO_MANIFEST_DIR=str(root),CARGO_MANIFEST_PATH=str(root/'Cargo.toml'),
        CARGO_PKG_NAME=name,CARGO_PKG_VERSION='1.2.59' if name=='cc' else '0.0.0',
        DYLD_FALLBACK_LIBRARY_PATH=str(host)+':'+os.environ['DYLD_FALLBACK_LIBRARY_PATH'])
    args=['--crate-name',name,'--edition=2021',str(source),'--crate-type','lib','--emit=dep-info,metadata,link','--out-dir',str(dest),*extra]
    result=subprocess.run([env['RUSTC_WRAPPER'],env['RUSTC'],*args],cwd=root,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    assert result.returncode==0,result.stderr
    emit({'reason':'compiler-artifact','package_id':pkg,'target':{'src_path':str(source),'name':name,'kind':['lib'],'crate_types':['lib']},
          'features':[],'filenames':[str(dest/('lib'+name+'.rlib')),str(dest/('lib'+name+'.rmeta'))],'executable':None,'fresh':False})
compile('cc',session/'vendor/cc/src/lib.rs','registry+https://github.com/rust-lang/crates.io-index#cc@1.2.59',host)
read,write=os.pipe();other_read,other_write=os.pipe();opened=[read,write,other_read,other_write]
original_stats=[os.fstat(fd).st_ino for fd in (read,write)]
native_loader=':'.join(map(str,(target/'debug',host,pathlib.Path(os.environ['DYLD_FALLBACK_LIBRARY_PATH']).parent/'lib/rustlib/x86_64-apple-darwin/lib',pathlib.Path(os.environ['DYLD_FALLBACK_LIBRARY_PATH']))))
results=[]
def family(name,ordinal):
    root=session/'vendor'/name;version='0.17.14' if name=='ring' else '0.1.30';major,minor,patch=version.split('.')
    out=target/'x86_64-apple-darwin/debug/build'/(name+'-0123456789abcdef')/'out';out.mkdir(parents=True,exist_ok=True)
    features=['alloc','default','dev_urandom_fallback','std'] if name=='ring' else []
    env=dict(base,CARGO=argv[0] if False else os.environ.get('CARGO',sys.argv[0]),CARGO_MANIFEST_DIR=str(root),CARGO_MANIFEST_PATH=str(root/'Cargo.toml'),
        CARGO_PKG_NAME=name,CARGO_PKG_VERSION=version,CARGO_PKG_VERSION_MAJOR=major,CARGO_PKG_VERSION_MINOR=minor,CARGO_PKG_VERSION_PATCH=patch,CARGO_PKG_VERSION_PRE='',
        HOST='x86_64-apple-darwin',TARGET='x86_64-apple-darwin',CARGO_CFG_TARGET_ARCH='x86_64',CARGO_CFG_TARGET_OS='macos',CARGO_CFG_TARGET_ENV='',
        CARGO_CFG_TARGET_ENDIAN='little',CARGO_CFG_TARGET_VENDOR='apple',CARGO_CFG_TARGET_POINTER_WIDTH='64',CARGO_CFG_TARGET_FAMILY='unix',
        CARGO_CFG_TARGET_ABI='',CARGO_CFG_UNIX='',CARGO_CFG_FEATURE=','.join(features),DEBUG='true',OPT_LEVEL='0',PROFILE='debug',
        CARGO_CFG_TARGET_FEATURE='cmpxchg16b,fxsr,sse,sse2,sse3,sse4.1,ssse3',CARGO_CFG_TARGET_HAS_ATOMIC='128,16,32,64,8,ptr',
        CARGO_CFG_DEBUG_ASSERTIONS='',CARGO_CFG_PANIC='unwind',
        OUT_DIR=str(out),LC_CTYPE='C.UTF-8',DYLD_FALLBACK_LIBRARY_PATH=native_loader,
        CARGO_MAKEFLAGS=f'-j --jobserver-fds={read},{write} --jobserver-auth={read},{write}')
    for k in ('LC_ALL','ZERO_AR_DATE'):env.pop(k,None)
    env.update({'CARGO_FEATURE_'+f.upper().replace('-','_'):'1' for f in features})
    if name=='ring':env['CARGO_MANIFEST_LINKS']='ring_core_0_17_14_'
    source=out/(str(ordinal)+'detect_compiler_family.c');source.write_bytes((session/'vendor/cc/src/detect_compiler_family.c').read_bytes())
    args=['-E',str(source)];role='CC';saved={}
    if CASE=='source_post':saved[root/'Cargo.toml']=(root/'Cargo.toml').read_bytes()
    if CASE.startswith('reject_'):
        choice=CASE[7:]
        if choice=='name':env['CARGO_PKG_NAME']='unknown'
        if choice=='version':env['CARGO_PKG_VERSION']='0.0.0'
        if choice=='component':env['CARGO_PKG_VERSION_PATCH']='0'
        if choice=='manifest':env['CARGO_MANIFEST_PATH']=str(app/'Cargo.toml')
        if choice=='manifest_absent':env.pop('CARGO_MANIFEST_PATH')
        if choice=='labels_absent':env.pop('CARGO_PKG_NAME')
        if choice=='features':env['CARGO_FEATURE_UNKNOWN']='1'
        if choice=='locale':env['LC_ALL']='C';env.pop('LC_CTYPE')
        if choice=='out':env['OUT_DIR']=str(target)
        if choice=='target':env['CARGO_CFG_TARGET_ARCH']='aarch64'
        if choice=='branch':env['CARGO_CFG_MIRI']=''
        if choice=='fd_missing':env.pop('CARGO_MAKEFLAGS')
        if choice=='fd_reversed':env['CARGO_MAKEFLAGS']=f'-j --jobserver-fds={write},{read} --jobserver-auth={write},{read}'
        if choice=='fd_foreign':env['CARGO_MAKEFLAGS']=f'-j --jobserver-fds={read},{other_write} --jobserver-auth={read},{other_write}'
        if choice=='fd_closed':env['CARGO_MAKEFLAGS']='-j --jobserver-fds=500,501 --jobserver-auth=500,501'
        if choice=='literal':source.write_bytes(b'X'*206)
        if choice=='extent':source.write_bytes(source.read_bytes()+b'X')
        if choice=='hardlink':copy=out/'literal-copy';copy.write_bytes(source.read_bytes());source.unlink();os.link(copy,source)
        if choice=='symlink':source.unlink();source.symlink_to(session/'vendor/cc/src/detect_compiler_family.c')
        if choice=='overflow':source=out/('18446744073709551616detect_compiler_family.c');source.write_bytes((session/'vendor/cc/src/detect_compiler_family.c').read_bytes());args=['-E',str(source)]
        if choice=='source':path=root/'build.rs';saved[path]=path.read_bytes();path.chmod(0o644);path.write_bytes(b'TEST_CODE_drift')
        if choice=='unknown':root=app;env['CARGO_MANIFEST_DIR']=str(app)
        if choice=='compile':args=['-c',str(root/'build.rs'),'-o',str(out/'foreign.o')]
        if choice=='archive':role='AR';args=['cqD',str(out/'foreign.a'),str(out/'foreign.o')]
        if choice=='retry':args=['-E','--',str(source)]
        if choice=='retry_reordered':args=['--','-E',str(source)]
        if choice=='extra_help':args=['-?','--version']
        if choice=='attached_help':args=['-?--version']
        if choice=='extra_version':args=['--version','-?']
        if choice=='wrong_role':role='AR';args=['-?']
    def execute(raw):
        result=subprocess.run([env[role],*raw],cwd=root,env=env,pass_fds=tuple(opened),stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        results.append({'name':name,'args':raw,'status':result.returncode,'stdout_hex':result.stdout.hex(),'stderr_hex':result.stderr.hex()})
        return result
    result=execute(args)
    if CASE=='duplicate_e':execute(args)
    if CASE.startswith('warning_') and result.returncode==3:execute(['-E','--',str(source)])
    if not CASE.startswith('reject_'):
        execute(['-?']);execute(['--version'])
    for path,body in saved.items():path.write_bytes(body);path.chmod(0o444)
    if CASE not in ('post_missing','post_change','source_post','survive') and source.exists() and not source.is_symlink():source.unlink()
if CASE=='concurrent':
    threads=[threading.Thread(target=family,args=('ring',i)) for i in (10,11)]
    for t in threads:t.start()
    for t in threads:t.join()
else:
    family('ring',10);family('psm',11)
assert [os.fstat(fd).st_ino for fd in (read,write)]==original_stats
for fd in opened:os.close(fd)
(session/'foreign-forwarded.json').write_text(json.dumps(results))
paths=list((session/'foreign-native-invocations').glob('*/receipt.json'))
if CASE=='request_only':paths[0].unlink()
if CASE in ('snapshot_changed','snapshot_missing'):
    p=next(p for p in paths if 'input_post' in json.loads(p.read_text()));r=json.loads(p.read_text());snapshot=p.parent/r['input_post']['snapshot']
    snapshot.unlink() if CASE=='snapshot_missing' else snapshot.write_bytes(b'TEST_CODE_changed')
cc_path=next(p for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('source')==str(session/'vendor/cc/src/lib.rs'))
if CASE in ('borrowed_request','borrowed_initial','borrowed_target','borrowed_context'):
    leaf='request.json' if CASE=='borrowed_request' else 'invocation.json';p=cc_path.parent/leaf;r=json.loads(p.read_text())
    if CASE=='borrowed_request':r['cwd_hex']=os.fsencode(str(app)).hex()
    elif CASE=='borrowed_initial':r['argv_hex'][0]=b'/foreign'.hex()
    elif CASE=='borrowed_target':r['role']='Target'
    else:r['context']={'kind':'DirectCargoProbe'}
    p.write_text(json.dumps(r))
if CASE.startswith('ownership_'):
    p=next(p for p in paths if 'input_post' in json.loads(p.read_text()));native=json.loads(p.read_text());snapshot=p.parent/native['input_post']['snapshot']
    copied=target/'copied-native.bin';copied.write_bytes(snapshot.read_bytes());r=json.loads(cc_path.read_text());kind=CASE[10:]
    value=str(snapshot if kind=='snapshot' else copied)
    if kind in ('source','snapshot','relative'):
        if kind=='relative':value=os.path.relpath(value,session/'vendor/cc')
        for o in r['outputs']:
            if o['kind']=='dep-info':o['dep_info']['paths'].append(value)
    elif kind=='output':r['declared_outputs'].append({'path':value,'kind':'link'})
    elif kind=='extern':r['externs'].append({'name':'foreign','path':value})
    elif kind=='artifact':emit({'reason':'compiler-artifact','package_id':'TEST_CODE_app','target':{'src_path':str(app/'src/lib.rs'),'name':'stock_analysis','kind':['lib'],'crate_types':['lib']},'features':[],'filenames':[value],'executable':None,'fresh':False})
    cc_path.write_text(json.dumps(r))
stream_controls=[]
if CASE.startswith('stream_') or CASE=='forward_fault':
    # Real wrapper capture first, then ownership claims and independent mutations.
    specifications=[tuple(CASE.split('_')[1:])] if CASE!='forward_fault' else [
        ('stdout','output','deleted'),('stderr','extern','mutated')]
    for stream,role,cut in specifications:
        request_cut=None
        invalid_state=cut=='deletedinvalidstate'
        if invalid_state:cut='deleted'
        if cut in ('mutatedmissingrequest','deletedcorruptrequest'):
            request_cut='missing' if cut=='mutatedmissingrequest' else 'corrupt'
            cut='mutated' if request_cut=='missing' else 'deleted'
        input_copy=stream=='input'
        actual='input_post' if input_copy else 'stderr' if stream=='empty' else stream
        wanted='CompilerFamilyHelpProbe' if actual=='stderr' and stream!='empty' else 'CompilerFamilyFileProbe'
        p=next(p for p in paths if p.is_file() and json.loads(p.read_text()).get('operation',{}).get('class')==wanted)
        observed=json.loads(p.read_text());raw=p.parent/(observed[actual]['snapshot'] if input_copy else actual+'.raw');body=raw.read_bytes()
        assert bool(body)==(stream!='empty')
        retained=observed[actual]['sha256'] if input_copy else observed[actual+'_sha256'];assert retained==hashlib.sha256(body).hexdigest()
        copied=target/('copied-stream-'+stream+'-'+role+'.bin');copied.write_bytes(body)
        if role=='generated':
            out=target/'x86_64-apple-darwin/debug/build/stock_analysis-2222222222222222/out';out.mkdir(parents=True)
            copied=out/'copied-stream.bin';copied.write_bytes(body);(out/'ordinary.txt').write_bytes(b'TEST_CODE_ordinary_generated')
            dest=target/'debug/build/stock_analysis-1111111111111111';source=app/'build.rs'
            env=dict(base,CARGO_MANIFEST_DIR=str(app),CARGO_MANIFEST_PATH=str(app/'Cargo.toml'),CARGO_PKG_NAME='stock_analysis',
                     CARGO_PKG_VERSION='0.0.0',DYLD_FALLBACK_LIBRARY_PATH=str(host)+':'+os.environ['DYLD_FALLBACK_LIBRARY_PATH'])
            args=['--crate-name','build_script_build','--edition=2021',str(source),'--crate-type','bin','--emit=dep-info,link','--out-dir',str(dest)]
            result=subprocess.run([env['RUSTC_WRAPPER'],env['RUSTC'],*args],cwd=app,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
            assert result.returncode==0,result.stderr
            emit({'reason':'compiler-artifact','package_id':'TEST_CODE_app','target':{'src_path':str(source),'name':'build_script_build',
                  'kind':['custom-build'],'crate_types':['bin']},'features':[],'filenames':[str(dest/'build_script_build')],'executable':None,'fresh':False})
            emit({'reason':'build-script-executed','package_id':'TEST_CODE_app','out_dir':str(out),'linked_libs':[],'linked_paths':[],'cfgs':[],'env':[]})
        r=json.loads(cc_path.read_text());value=str(copied)
        if role=='source':
            for output in r['outputs']:
                if output['kind']=='dep-info':output['dep_info']['paths'].append(value)
        elif role=='output':r['declared_outputs'].append({'path':value,'kind':'link'})
        elif role=='extern':r['externs'].append({'name':'copied_stream','path':value})
        cc_path.write_text(json.dumps(r))
        control={'stream':stream,'role':role,'cut':cut,'path':value,'operation_id':p.parent.name,
                 'raw_path':str(raw),'retained_sha256':retained,'body_hex':body.hex(),'original_protocol_state':observed['protocol_state']}
        matching=[q for q in (session/'foreign-native-invocations').glob('*/*.raw') if q.read_bytes()==body]
        control['matching_raw_paths']=list(map(str,matching))
        annotations=[q for q in paths if q.is_file() and (json.loads(q.read_text()).get('input_post',{}).get('sha256')
                     if input_copy else json.loads(q.read_text()).get(actual+'_sha256'))==retained]
        control['matching_receipt_paths']=list(map(str,annotations))
        if cut=='mutated':
            for q in matching:q.write_bytes(b'TEST_CODE_stream_drift_'+actual.encode())
        elif cut=='deleted':
            for q in matching:q.unlink()
        elif cut=='annotation':
            for q in annotations:
                data=json.loads(q.read_text());data.pop(actual+'_sha256');q.write_text(json.dumps(data))
        elif cut=='requestonly':
            for q in annotations:q.unlink()
        if invalid_state:
            control['invalid_snapshot_field']='input_pre'
            for q in annotations:
                data=json.loads(q.read_text());data['input_pre']='TEST_CODE_invalid_state';q.write_text(json.dumps(data))
        if request_cut:
            control['request_cut']=request_cut
            control['matching_request_paths']=[str(q.parent/'request.json') for q in annotations]
            for name in control['matching_request_paths']:
                q=pathlib.Path(name)
                if request_cut=='missing':q.unlink()
                else:q.write_bytes(b'{TEST_CODE_invalid_request')
        stream_controls.append(control)
compile('stock_analysis',app/'src/lib.rs','TEST_CODE_app',deps,['--target','x86_64-apple-darwin'])
for control in stream_controls:
    if control['role']=='artifact':
        # Claim ordinary app products and their real Cargo artifact/selection edge.
        p=next(p for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('source')==str(app/'src/lib.rs'))
        r=json.loads(p.read_text());body=bytes.fromhex(control['body_hex']);changed=[]
        for output in r['outputs']:
            if output['kind'] in ('link','metadata'):
                pathlib.Path(output['path']).write_bytes(body);output['sha256']=control['retained_sha256'];changed.append(output['path'])
        p.write_text(json.dumps(r));control['artifact_paths']=changed
(session/'stream-copy-controls.json').write_text(json.dumps(stream_controls))
emit({'reason':'build-finished','success':True})
'''

# Stage B fixtures use bound bundled2 raw data, not production classifier output.
E2_SOURCE_DATA = {'ring': [('crypto/cpu_intel.c', 'a4019cc0736b0423-cpu_intel.o'), ('crypto/crypto.c', 'a4019cc0736b0423-crypto.o'), ('crypto/curve25519/curve25519.c', '25ac62e5b3c53843-curve25519.o'), ('crypto/curve25519/curve25519_64_adx.c', '25ac62e5b3c53843-curve25519_64_adx.o'), ('crypto/fipsmodule/aes/aes_nohw.c', '0bbbd18bda93c05b-aes_nohw.o'), ('crypto/fipsmodule/bn/montgomery.c', '00c879ee3285a50d-montgomery.o'), ('crypto/fipsmodule/bn/montgomery_inv.c', '00c879ee3285a50d-montgomery_inv.o'), ('crypto/fipsmodule/ec/ecp_nistz.c', 'a0330e891e733f4e-ecp_nistz.o'), ('crypto/fipsmodule/ec/gfp_p256.c', 'a0330e891e733f4e-gfp_p256.o'), ('crypto/fipsmodule/ec/gfp_p384.c', 'a0330e891e733f4e-gfp_p384.o'), ('crypto/fipsmodule/ec/p256-nistz.c', 'a0330e891e733f4e-p256-nistz.o'), ('crypto/fipsmodule/ec/p256.c', 'a0330e891e733f4e-p256.o'), ('crypto/limbs/limbs.c', 'aaa1ba3e455ee2e1-limbs.o'), ('crypto/mem.c', 'a4019cc0736b0423-mem.o'), ('crypto/poly1305/poly1305.c', 'd5a9841f3dc6e253-poly1305.o'), ('pregenerated/aes-gcm-avx2-x86_64-macosx.S', 'c322a0bcc369f531-aes-gcm-avx2-x86_64-macosx.o'), ('pregenerated/aesni-gcm-x86_64-macosx.S', 'c322a0bcc369f531-aesni-gcm-x86_64-macosx.o'), ('pregenerated/aesni-x86_64-macosx.S', 'c322a0bcc369f531-aesni-x86_64-macosx.o'), ('pregenerated/chacha-x86_64-macosx.S', 'c322a0bcc369f531-chacha-x86_64-macosx.o'), ('pregenerated/chacha20_poly1305_x86_64-macosx.S', 'c322a0bcc369f531-chacha20_poly1305_x86_64-macosx.o'), ('pregenerated/ghash-x86_64-macosx.S', 'c322a0bcc369f531-ghash-x86_64-macosx.o'), ('pregenerated/p256-x86_64-asm-macosx.S', 'c322a0bcc369f531-p256-x86_64-asm-macosx.o'), ('pregenerated/sha256-x86_64-macosx.S', 'c322a0bcc369f531-sha256-x86_64-macosx.o'), ('pregenerated/sha512-x86_64-macosx.S', 'c322a0bcc369f531-sha512-x86_64-macosx.o'), ('pregenerated/vpaes-x86_64-macosx.S', 'c322a0bcc369f531-vpaes-x86_64-macosx.o'), ('pregenerated/x86_64-mont-macosx.S', 'c322a0bcc369f531-x86_64-mont-macosx.o'), ('pregenerated/x86_64-mont5-macosx.S', 'c322a0bcc369f531-x86_64-mont5-macosx.o'), ('third_party/fiat/asm/fiat_curve25519_adx_mul.S', 'e165cd818145c705-fiat_curve25519_adx_mul.o'), ('third_party/fiat/asm/fiat_curve25519_adx_square.S', 'e165cd818145c705-fiat_curve25519_adx_square.o')], 'psm': [('src/arch/x86_64.s', '4f9a91766097c4c5-x86_64.o')]}
E2_PREFIX_DATA = {'ring': ['-O0', '-ffunction-sections', '-fdata-sections', '-fPIC', '-g', '-gdwarf-2', '-fno-omit-frame-pointer', '-m64', '--target=x86_64-apple-macosx', '-mmacosx-version-min=26.5', '-I', '$ROOT/include', '-I', '$ROOT/pregenerated', '-Wall', '-Wextra', '-fvisibility=hidden', '-std=c1x', '-Wall', '-Wbad-function-cast', '-Wcast-align', '-Wcast-qual', '-Wconversion', '-Wmissing-field-initializers', '-Wmissing-include-dirs', '-Wnested-externs', '-Wredundant-decls', '-Wshadow', '-Wsign-compare', '-Wsign-conversion', '-Wstrict-prototypes', '-Wundef', '-Wuninitialized', '-gfull', '-DNDEBUG'], 'psm': ['-O0', '-ffunction-sections', '-fdata-sections', '-fPIC', '-g', '-gdwarf-2', '-fno-omit-frame-pointer', '-m64', '--target=x86_64-apple-macosx', '-mmacosx-version-min=26.5', '-Wall', '-Wextra', '-xassembler-with-cpp', '-DCFG_TARGET_OS_macos', '-DCFG_TARGET_ARCH_x86_64', '-DCFG_TARGET_ENV_']}
E2_PRIVATE_HEADER_MEMBERS = ('ring/crypto/curve25519/curve25519_tables.h', 'ring/crypto/curve25519/internal.h', 'ring/crypto/fipsmodule/bn/internal.h', 'ring/crypto/fipsmodule/ec/ecp_nistz.h', 'ring/crypto/fipsmodule/ec/ecp_nistz384.h', 'ring/crypto/fipsmodule/ec/ecp_nistz384.inl', 'ring/crypto/fipsmodule/ec/p256-nistz-table.h', 'ring/crypto/fipsmodule/ec/p256-nistz.h', 'ring/crypto/fipsmodule/ec/p256_shared.h', 'ring/crypto/fipsmodule/ec/p256_table.h', 'ring/crypto/fipsmodule/ec/util.h', 'ring/crypto/internal.h', 'ring/crypto/limbs/limbs.h', 'ring/crypto/limbs/limbs.inl', 'ring/third_party/fiat/curve25519_32.h', 'ring/third_party/fiat/curve25519_64.h', 'ring/third_party/fiat/curve25519_64_adx.h', 'ring/third_party/fiat/curve25519_64_msvc.h', 'ring/third_party/fiat/p256_32.h', 'ring/third_party/fiat/p256_64.h', 'ring/third_party/fiat/p256_64_msvc.h')
E2_NATIVE = E1_NATIVE.replace("if args[0]=='-E':", r"""if '-c' in args:
    output=pathlib.Path(args[args.index('-o')+1]);source=pathlib.Path(args[-1])
    os.write(1,b'TEST_CODE_compile_stdout\n');os.write(2,b'TEST_CODE_compile_stderr\n')
    if case!='object_missing':output.write_bytes(b'TEST_CODE_object:'+source.name.encode())
    if case=='source_post':source.chmod(0o644);source.write_bytes(b'X')
    if case=='include_post':
        header=pathlib.Path(env['CARGO_MANIFEST_DIR'])/'include/TEST_CODE.h'
        if header.exists():header.chmod(0o644);header.write_bytes(b'X')
    if case=='private_header_post':
        header=pathlib.Path(env['CARGO_MANIFEST_DIR'])/'crypto/internal.h'
        if header.exists():header.chmod(0o644);header.write_bytes(b'TEST_CODE_private_header_post')
    if case=='signal':os.kill(os.getpid(),signal.SIGTERM)
    sys.exit(7 if case in ('nonzero','sticky') else 0)
if args[0]=='-E':
    if case in ('source_post','signal'):case='normal'""").replace("sys.exit(0 if case=='help_zero' else 9)", "sys.exit(0 if case=='family_msvc' else 1)")
E2_NATIVE = E2_NATIVE.replace("case in ('version_zig','version_nonzero')", "case=='family_zig'").replace("else:os.write(1,b'\"clang\" \"gcc\"\\n')", "else:os.write(1,b'\"gcc\"\\n' if case=='family_label' else b'\"clang\" \"gcc\"\\n')")
E2_COMPILE_CARGO = r'''
    if source.exists():source.unlink()
    if CASE=='family_resurrect':source.write_bytes((session/'vendor/cc/src/detect_compiler_family.c').read_bytes())
    if CASE=='family_duplicate':execute(['-?'])
    family_paths=[p for p in (session/'foreign-native-invocations').glob('*/receipt.json')
                  if json.loads(p.read_text()).get('context',{}).get('manifest')==str(root)]
    if CASE=='family_pending':
        p=next(p for p in family_paths if json.loads(p.read_text())['operation']['class']=='CompilerFamilyHelpProbe');p.unlink()
    if CASE in ('family_stream','family_label','family_control','family_fd'):
        p=next(p for p in family_paths if json.loads(p.read_text())['operation']['class']=='CompilerFamilyFileProbe')
        receipt=json.loads(p.read_text())
        if CASE=='family_stream':(p.parent/'stdout.raw').write_bytes(b'TEST_CODE_tampered_family_stream')
        if CASE=='family_label':receipt['source_semantics']['markers']['clang']=True
        if CASE=='family_control':receipt['controls_pre']['owner_source_sha256']='0'*64
        if CASE=='family_fd':receipt['jobserver_return']={}
        if CASE!='family_stream':p.write_text(json.dumps(receipt))
    env['LC_ALL']='C';env.pop('LC_CTYPE',None)
    selected=E2_SOURCE_DATA[name] if CASE in ('normal','parallel') else E2_SOURCE_DATA[name][:2 if CASE=='sticky' else 1]
    def object_call(specification):
        member,basename=specification;raw=str(root/member) if name=='ring' else member
        output=out/basename;args=[v.replace('$ROOT',str(root)) for v in E2_PREFIX_DATA[name]]+['-o',str(output),'-c',raw]
        saved={}
        if CASE=='template_source':args[-1]=str(root/'build.rs')
        if CASE=='template_flag':args[0]='-O2'
        if CASE=='template_order':args[0],args[1]=args[1],args[0]
        if CASE=='template_prefix':args[-3]=str(out/('0000000000000000-'+basename.split('-',1)[1]))
        if CASE=='template_suffix':args[-3]=str(output.with_suffix('.a'))
        if CASE=='template_escape':args[-3]=str(target/basename)
        if CASE=='template_locale':env['LC_CTYPE']='C.UTF-8'
        if CASE=='template_fd':env['CARGO_MAKEFLAGS']=f'-j --jobserver-fds={write},{read} --jobserver-auth={write},{read}'
        if CASE=='template_existing':output.write_bytes(b'TEST_CODE_existing_object')
        if CASE in ('private_header_pre','private_header_copy_pre'):
            path=root/'crypto/internal.h'
            if path.exists():
                if CASE=='private_header_pre':saved[path]=path.read_bytes()
                path.chmod(0o644);path.write_bytes(b'TEST_CODE_private_header_pre')
        if CASE in ('template_hash','template_include','template_helper'):
            path=(root/member if CASE=='template_hash' else root/'include/TEST_CODE.h' if CASE=='template_include' else session/'vendor/cc/src/target/apple.rs')
            if path.exists():saved[path]=path.read_bytes();path.chmod(0o644);path.write_bytes(b'TEST_CODE_changed')
        execute(args)
        if CASE=='template_duplicate':execute(args)
        for path,body in saved.items():path.write_bytes(body);path.chmod(0o444)
    if CASE=='parallel':
        import concurrent.futures
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:list(pool.map(object_call,selected))
    else:
        for specification in selected:object_call(specification)
    if CASE=='archive':
        role='AR';execute(['cqD',str(out/'unqualified.a'),str(out/selected[0][1])]);role='CC'
'''
E2_CARGO = E1_CARGO.replace("base=dict(os.environ,", "E2_SOURCE_DATA="+repr(E2_SOURCE_DATA)+"\nE2_PREFIX_DATA="+repr(E2_PREFIX_DATA)+"\nbase=dict(os.environ,")
E2_CARGO = E2_CARGO.replace("execute(['-?']);execute(['--version'])", "execute(['-?'])\n        if CASE!='family_missing':execute(['--version'])")
E2_CARGO = E2_CARGO.replace("    for path,body in saved.items():path.write_bytes(body);path.chmod(0o444)", E2_COMPILE_CARGO+"\n    for path,body in saved.items():path.write_bytes(body);path.chmod(0o444)")
E2_CARGO = E2_CARGO.replace("paths=list((session/'foreign-native-invocations').glob('*/receipt.json'))", r'''
paths=list((session/'foreign-native-invocations').glob('*/receipt.json'))
object_paths=[p for p in paths if json.loads(p.read_text()).get('operation',{}).get('class')=='CompilerObjectCompile'
              and json.loads(p.read_text()).get('output_post',{}).get('exists')]
if CASE in ('object_mutated','object_snapshot_missing','object_request_only'):
    p=object_paths[0];receipt=json.loads(p.read_text())
    if CASE=='object_mutated':pathlib.Path(receipt['output_post']['path']).write_bytes(b'TEST_CODE_object_drift')
    if CASE=='object_snapshot_missing':(p.parent/receipt['output_post']['snapshot']).unlink()
    if CASE=='object_request_only':p.unlink()
if CASE in ('private_header_copy_current','private_header_copy_retained','private_header_copy_pre','private_header_copy_request_only'):
    header=session/'vendor/ring/crypto/internal.h';body=header.read_bytes();retained=hashlib.sha256(body).hexdigest()
    copy=target/'copied-private-header.bin';copy.write_bytes(body)
    cc_path=next(p for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('source')==str(session/'vendor/cc/src/lib.rs'))
    cc=json.loads(cc_path.read_text());ring_paths=[p for p in paths if json.loads(p.read_text()).get('operation',{}).get('class')=='CompilerObjectCompile'
                                               and json.loads(p.read_text())['context']['package_id'].endswith('#ring@0.17.14')]
    assert len(ring_paths)==1
    p=ring_paths[0];receipt=json.loads(p.read_text());member='ring/crypto/internal.h';expected=receipt['compile_input_declaration']['source_sha256'][member]
    assert receipt['compile_input_declaration']['state']=='DeclaredOnly'
    if CASE=='private_header_copy_pre':
        assert receipt['protocol_state']=='ProtocolRefused' and receipt['tool_result'] is None and receipt['failures']==['ForeignCompileSourcePin']
        assert 'compile_pins_pre' not in receipt and expected!=retained
        (p.parent/'request.json').write_bytes(b'{TEST_CODE_invalid_pre_private_request')
    else:
        assert all(receipt[k]['source_sha256'][member]==retained for k in ('compile_pins_pre','compile_pins_post','compile_pins_return'))
    if CASE in ('private_header_copy_retained','private_header_copy_request_only'):
        header.chmod(0o644);header.write_bytes(b'TEST_CODE_private_header_new_current')
        if CASE=='private_header_copy_retained':(p.parent/'request.json').write_bytes(b'{TEST_CODE_invalid_private_request')
        else:p.unlink()
        cc['externs'].append({'name':'private_header_copy','path':str(copy)})
    else:
        for o in cc['outputs']:
            if o['kind']=='dep-info':o['dep_info']['paths'].append(str(copy))
    cc_path.write_text(json.dumps(cc))
    (session/'private-header-copy-control.json').write_text(json.dumps({'member':member,'source':str(header),'copy':str(copy),
        'retained_sha256':retained,'expected_sha256':expected,'body_hex':body.hex(),'operation_id':p.parent.name,
        'role':'extern' if CASE in ('private_header_copy_retained','private_header_copy_request_only') else 'source'}))
if CASE.startswith('ownership_object_'):
    p=object_paths[0];receipt=json.loads(p.read_text());body=(p.parent/receipt['output_post']['snapshot']).read_bytes()
    copy=target/'copied-object.bin';copy.write_bytes(body)
    cc_path=next(p for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('source')==str(session/'vendor/cc/src/lib.rs'))
    cc=json.loads(cc_path.read_text());kind=CASE[len('ownership_object_'):]
    if kind=='source':
        for o in cc['outputs']:
            if o['kind']=='dep-info':o['dep_info']['paths'].append(str(copy))
    elif kind=='extern':cc['externs'].append({'name':'foreign_object','path':str(copy)})
    elif kind=='output':cc['declared_outputs'].append({'path':str(copy),'kind':'link'})
    cc_path.write_text(json.dumps(cc))
    if kind=='retained':
        retained=receipt['output_post']['sha256']
        for q in (session/'foreign-native-invocations').glob('*/*.raw'):
            if hashlib.sha256(q.read_bytes()).hexdigest()==retained:q.unlink()
        pathlib.Path(receipt['output_post']['path']).unlink()
        (p.parent/'request.json').write_bytes(b'{TEST_CODE_bad_request')
        for o in cc['outputs']:
            if o['kind']=='dep-info':o['dep_info']['paths'].append(str(copy))
        cc_path.write_text(json.dumps(cc))
''')


E3_SELECTED = {'ring': [('crypto/curve25519/curve25519.c', '25ac62e5b3c53843-curve25519.o'), ('crypto/fipsmodule/aes/aes_nohw.c', '0bbbd18bda93c05b-aes_nohw.o'), ('crypto/fipsmodule/bn/montgomery.c', '00c879ee3285a50d-montgomery.o'), ('crypto/fipsmodule/bn/montgomery_inv.c', '00c879ee3285a50d-montgomery_inv.o'), ('crypto/fipsmodule/ec/ecp_nistz.c', 'a0330e891e733f4e-ecp_nistz.o'), ('crypto/fipsmodule/ec/gfp_p256.c', 'a0330e891e733f4e-gfp_p256.o'), ('crypto/fipsmodule/ec/gfp_p384.c', 'a0330e891e733f4e-gfp_p384.o'), ('crypto/fipsmodule/ec/p256.c', 'a0330e891e733f4e-p256.o'), ('crypto/limbs/limbs.c', 'aaa1ba3e455ee2e1-limbs.o'), ('crypto/mem.c', 'a4019cc0736b0423-mem.o'), ('crypto/poly1305/poly1305.c', 'd5a9841f3dc6e253-poly1305.o'), ('crypto/crypto.c', 'a4019cc0736b0423-crypto.o'), ('crypto/cpu_intel.c', 'a4019cc0736b0423-cpu_intel.o'), ('crypto/curve25519/curve25519_64_adx.c', '25ac62e5b3c53843-curve25519_64_adx.o'), ('third_party/fiat/asm/fiat_curve25519_adx_mul.S', 'e165cd818145c705-fiat_curve25519_adx_mul.o'), ('third_party/fiat/asm/fiat_curve25519_adx_square.S', 'e165cd818145c705-fiat_curve25519_adx_square.o')], 'psm': [('src/arch/x86_64.s', '4f9a91766097c4c5-x86_64.o')]}

# Stage C synthetic append bytes are not an archive format/consumer qualification.
E3_NATIVE_AR = r'''
import json,os,pathlib,signal,sys,uuid
args=sys.argv[1:];env=dict(os.environ);session=pathlib.Path(env['E1_SESSION']);case=env['E1_CASE']
pair=tuple(map(int,env['CARGO_MAKEFLAGS'].split('--jobserver-fds=')[1].split()[0].split(',')))
hits=session/'archive-entry';hits.mkdir(exist_ok=True)
(hits/uuid.uuid4().hex).write_text(json.dumps({'argv':args,'cwd':str(pathlib.Path.cwd()),'environment':env,
    'fds':list(pair),'inodes':[os.fstat(fd).st_ino for fd in pair],'stdin_eof':sys.stdin.buffer.read()==b''}))
archive=pathlib.Path(args[1]);body=(b'TEST_CODE_partial_archive:' if case=='ar_partial' and args[0]=='cqD' else b'TEST_CODE_append_archive:')+env['CARGO_PKG_NAME'].encode()+b'\n'
if case!='ar_missing':archive.write_bytes((archive.read_bytes() if archive.exists() else b'')+body)
os.write(1,b'TEST_CODE_archive_stdout\n');os.write(2,b'TEST_CODE_archive_stderr\n')
if case=='ar_input_post':pathlib.Path(args[2]).write_bytes(b'TEST_CODE_changed_member')
if case=='ar_control_post':
    p=session/'owner.json';o=json.loads(p.read_text());o['native_launchers']['ar']['sha256']='0'*64;p.write_text(json.dumps(o))
if case=='ar_signal':os.kill(os.getpid(),signal.SIGTERM)
sys.exit(7 if case=='ar_partial' and args[0]=='cqD' else 0)
'''
E3_ARCHIVE_CARGO = r'''
    env.pop('LC_ALL',None);env['LC_CTYPE']='C.UTF-8';env['ZERO_AR_DATE']='1';role='AR'
    archive=out/('libring_core_0_17_14_.a' if name=='ring' else 'libpsm_s.a')
    raw=['cqD',str(archive),*[str(out/basename) for _,basename in selected]]
    if CASE=='ar_env_locale':env['LC_ALL']='C';env.pop('LC_CTYPE')
    if CASE=='ar_env_zero':env.pop('ZERO_AR_DATE')
    if CASE=='ar_fd_foreign':env['CARGO_MAKEFLAGS']=f'-j --jobserver-fds={read},{other_write} --jobserver-auth={read},{other_write}'
    if CASE=='ar_member_order':raw[-1],raw[-2]=raw[-2],raw[-1]
    if CASE=='ar_extra_member':raw.append(str(out/'unobserved.o'))
    if CASE=='ar_existing':archive.write_bytes(b'TEST_CODE_unowned_archive')
    if CASE=='ar_cq_without_probe':raw[0]='cq'
    if CASE=='ar_member_missing':(out/selected[-1][1]).unlink()
    saved_owner=(session/'owner.json').read_bytes()
    result=execute(raw)
    if CASE in ('ar_bad_operation_none','ar_bad_operation_list'):
        p=next(p for p in (session/'foreign-native-invocations').glob('*/receipt.json')
               if json.loads(p.read_text()).get('role')=='ar' and json.loads(p.read_text())['context']['manifest']==str(root))
        r=json.loads(p.read_text());r['operation']=None if CASE=='ar_bad_operation_none' else [];p.write_text(json.dumps(r))
        execute(['cq',*raw[1:]])
    if CASE=='ar_partial' and result.returncode==7:execute(['cq',*raw[1:]])
    if CASE in ('ar_capture','ar_forward','ar_fd_return','ar_input_post'):execute(['cq',*raw[1:]])
    if CASE=='ar_control_post':(session/'owner.json').write_bytes(saved_owner)
    if CASE in ('ar_index','ar_test','ar_remaining'):
        extra=['sD',str(archive)] if CASE=='ar_index' else ['cqD',str(out/'libring_core_0_17_14__test.a'),str(out/'a4019cc0736b0423-constant_time_test.o')] if CASE=='ar_test' else ['cq',str(archive),str(out/'a0330e891e733f4e-p256-nistz.o')]
        execute(extra)
    role='CC'
'''
E3_COPY_CARGO = r'''
if CASE.startswith('ar_copy_'):
    p=next(p for p in paths if json.loads(p.read_text()).get('operation',{}).get('class')=='ArchiverFirstAppend'
           and json.loads(p.read_text())['context']['manifest']==str(session/'vendor/ring'))
    receipt=json.loads(p.read_text());body=(p.parent/receipt['archive_post']['snapshot']).read_bytes();retained=hashlib.sha256(body).hexdigest()
    copy=target/'copied-archive.bin';copy.write_bytes(body)
    cc_path=next(p for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('source')==str(session/'vendor/cc/src/lib.rs'))
    cc=json.loads(cc_path.read_text());kind=CASE[len('ar_copy_'):]
    if kind in ('source','retained','request_only'):
        next(o for o in cc['outputs'] if o['kind']=='dep-info')['dep_info']['paths'].append(str(copy))
    if kind=='extern':cc['externs'].append({'name':'foreign_archive','path':str(copy)})
    if kind=='output':cc['declared_outputs'].append({'path':str(copy),'kind':'link'})
    cc_path.write_text(json.dumps(cc))
    if kind=='retained':
        for q in (session/'foreign-native-invocations').glob('*/*.raw'):
            if hashlib.sha256(q.read_bytes()).hexdigest()==retained:q.unlink()
        for q in (session/'foreign-native-invocations').glob('*/receipt.json'):
            r=json.loads(q.read_text())
            if r.get('archive_post',{}).get('sha256')==retained:
                pathlib.Path(r['archive_post']['path']).unlink(missing_ok=True)
                (q.parent/'request.json').write_bytes(b'{TEST_CODE_bad_archive_request')
    if kind=='request_only':
        p.unlink()
        for q in (session/'foreign-native-invocations').glob('*/*.raw'):
            if hashlib.sha256(q.read_bytes()).hexdigest()==retained:q.unlink()
    (session/'archive-copy-control.json').write_text(json.dumps({'copy':str(copy),'retained_sha256':retained,
        'body_hex':body.hex(),'operation_id':p.parent.name,'cc_id':cc_path.parent.name,'cc_receipt_sha256':hashlib.sha256(cc_path.read_bytes()).hexdigest(),'kind':kind}))
'''


# Source-bound E-only fixture; no real Cargo/compiler/native or issuance path.
E4_FACTS = {'lz4-sys': ('1.11.1+lz4-1.10.0', ('1', '11', '1', ''), (), 'lz4', 4), 'zstd-sys': ('2.0.16+zstd.1.5.7', ('2', '0', '16', ''), ('legacy', 'std', 'zdict_builder'), 'zstd', 40)}
E4_NATIVE = r'''
import json,os,pathlib,signal,sys,uuid
args=sys.argv[1:];env=dict(os.environ);session=pathlib.Path(env['E4_SESSION']);case=env['E4_CASE'];name=env['CARGO_PKG_NAME']
assert args[:1]==['-E']
pair=tuple(map(int,env['CARGO_MAKEFLAGS'].split('--jobserver-fds=')[1].split()[0].split(',')))
hits=session/'foreign-entry';hits.mkdir(exist_ok=True)
(hits/uuid.uuid4().hex).write_text(json.dumps({'argv':args,'environment':env,'cwd':str(pathlib.Path.cwd()),
    'stdin_eof':sys.stdin.buffer.read()==b'','fds':list(pair),'inodes':[os.fstat(fd).st_ino for fd in pair]}))
source=pathlib.Path(args[-1])
if case=='post_missing':source.unlink()
if case=='post_change':source.write_bytes(b'TEST_CODE_changed_E_input')
if case=='pin_post':
    header=pathlib.Path(env['E4_HEADER']);header.chmod(0o644);header.write_bytes(b'TEST_CODE_post_header:'+name.encode())
if (case.startswith('warning_') or case=='predecessor_tamper') and '--' not in args:
    os.write(1 if case=='warning_stdout' else 2,b'-Wslash-u-filename\n'+name.encode()+b'\n');sys.exit(3)
os.write(1,b'"clang" "gcc" TEST_CODE_E4:'+name.encode()+b'\n');os.write(2,b'TEST_CODE_E4_stderr:'+name.encode()+b'\n')
if case=='control_post':
    path=session/'owner.json';o=json.loads(path.read_bytes());o['TEST_CODE_E4_drift']=name;path.write_text(json.dumps(o))
if case=='signal':os.kill(os.getpid(),signal.SIGTERM)
sys.exit(7 if case=='nonzero' else 0)
'''
E4_CARGO = r'''
import concurrent.futures,hashlib,json,os,pathlib,subprocess,sys,threading,time
CASE=__CASE__;argv=sys.argv[1:];app=pathlib.Path(argv[argv.index('--manifest-path')+1]).parent;session=app.parent
target=session/'target';host=target/'debug/deps';deps=target/'x86_64-apple-darwin/debug/deps'
base=dict(os.environ,E4_SESSION=str(session),E4_CASE=CASE,D1_SESSION=str(session),D1_CASE='normal',CARGO_ENCODED_RUSTFLAGS='')
def emit(e):print(json.dumps(e),flush=True)
def compile(name,source,pkg,dest,extra=()):
    root=source.parent.parent;env=dict(base,CARGO_MANIFEST_DIR=str(root),CARGO_MANIFEST_PATH=str(root/'Cargo.toml'),
        CARGO_PKG_NAME=name,CARGO_PKG_VERSION='1.2.59' if name=='cc' else '0.0.0',
        DYLD_FALLBACK_LIBRARY_PATH=str(host)+':'+os.environ['DYLD_FALLBACK_LIBRARY_PATH'])
    args=['--crate-name',name,'--edition=2021',str(source),'--crate-type','lib','--emit=dep-info,metadata,link','--out-dir',str(dest),*extra]
    result=subprocess.run([env['RUSTC_WRAPPER'],env['RUSTC'],*args],cwd=root,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    assert result.returncode==0,result.stderr
    emit({'reason':'compiler-artifact','package_id':pkg,'target':{'src_path':str(source),'name':name,'kind':['lib'],'crate_types':['lib']},
          'features':[],'filenames':[str(dest/('lib'+name+'.rlib')),str(dest/('lib'+name+'.rmeta'))],'executable':None,'fresh':False})
compile('cc',session/'vendor/cc/src/lib.rs','registry+https://github.com/rust-lang/crates.io-index#cc@1.2.59',host)
read,write=os.pipe();other_read,other_write=os.pipe();opened=[read,write,other_read,other_write]
original_stats=[os.fstat(fd).st_ino for fd in (read,write)]
native_loader=':'.join(map(str,(target/'debug',host,pathlib.Path(os.environ['DYLD_FALLBACK_LIBRARY_PATH']).parent/'lib/rustlib/x86_64-apple-darwin/lib',pathlib.Path(os.environ['DYLD_FALLBACK_LIBRARY_PATH']))))
E4_FACTS=__E4_FACTS__

results=[]
def family(name,ordinal):
    version,components,features,links,count=E4_FACTS[name];root=session/'vendor'/name
    out=target/'x86_64-apple-darwin/debug/build'/(name+'-0123456789abcdef')/'out';out.mkdir(parents=True,exist_ok=True)
    env=dict(base,CARGO=os.environ.get('CARGO',sys.argv[0]),CARGO_MANIFEST_DIR=str(root),CARGO_MANIFEST_PATH=str(root/'Cargo.toml'),
        CARGO_PKG_NAME=name,CARGO_PKG_VERSION=version,CARGO_PKG_VERSION_MAJOR=components[0],CARGO_PKG_VERSION_MINOR=components[1],
        CARGO_PKG_VERSION_PATCH=components[2],CARGO_PKG_VERSION_PRE=components[3],CARGO_MANIFEST_LINKS=links,
        HOST='x86_64-apple-darwin',TARGET='x86_64-apple-darwin',CARGO_CFG_TARGET_ARCH='x86_64',CARGO_CFG_TARGET_OS='macos',CARGO_CFG_TARGET_ENV='',
        CARGO_CFG_TARGET_ENDIAN='little',CARGO_CFG_TARGET_VENDOR='apple',CARGO_CFG_TARGET_POINTER_WIDTH='64',CARGO_CFG_TARGET_FAMILY='unix',
        CARGO_CFG_TARGET_ABI='',CARGO_CFG_UNIX='',CARGO_CFG_FEATURE=','.join(features),DEBUG='true',OPT_LEVEL='0',PROFILE='debug',
        CARGO_CFG_TARGET_FEATURE='cmpxchg16b,fxsr,sse,sse2,sse3,sse4.1,ssse3',CARGO_CFG_TARGET_HAS_ATOMIC='128,16,32,64,8,ptr',
        CARGO_CFG_DEBUG_ASSERTIONS='',CARGO_CFG_PANIC='unwind',OUT_DIR=str(out),LC_CTYPE='C.UTF-8',DYLD_FALLBACK_LIBRARY_PATH=native_loader,
        CARGO_MAKEFLAGS=f'-j --jobserver-fds={read},{write} --jobserver-auth={read},{write}')
    for k in ('LC_ALL','ZERO_AR_DATE'):env.pop(k,None)
    env.update({'CARGO_FEATURE_'+f.upper().replace('-','_'):'1' for f in features})
    header=root/('liblz4/lib/lz4.h' if name=='lz4-sys' else 'zstd/lib/common/allocations.h');env['E4_HEADER']=str(header)
    source=out/(str(ordinal)+'detect_compiler_family.c');source.write_bytes((session/'vendor/cc/src/detect_compiler_family.c').read_bytes())
    args=['-E',str(source)];role='CC'
    if CASE=='pin_pre':header.chmod(0o644);header.write_bytes(b'TEST_CODE_pre_header:'+name.encode())
    if CASE.startswith('reject_'):
        choice=CASE[len('reject_'):]
        if choice=='name':env['CARGO_PKG_NAME']='unknown'
        if choice=='version':env['CARGO_PKG_VERSION']=version.split('+')[0]
        if choice.startswith('component_'):env['CARGO_PKG_VERSION_'+choice[len('component_'):].upper()]='TEST_CODE_wrong'
        if choice=='links':env['CARGO_MANIFEST_LINKS']='unknown'
        if choice=='features':env['CARGO_FEATURE_UNKNOWN']='1'
        if choice=='manifest':env['CARGO_MANIFEST_PATH']=str(app/'Cargo.toml')
        if choice=='labels_absent':env.pop('CARGO_PKG_NAME')
        if choice=='out':env['OUT_DIR']=str(target)
        if choice=='out_label':env['OUT_DIR']=str(out.parent.parent/'unknown-0123456789abcdef'/'out');pathlib.Path(env['OUT_DIR']).mkdir(parents=True,exist_ok=True)
        if choice=='target':env['CARGO_CFG_TARGET_ARCH']='aarch64'
        if choice=='windows':env['CARGO_CFG_WINDOWS']=''
        if choice=='pkgconfig':env['ZSTD_SYS_USE_PKG_CONFIG']='1'
        if choice=='locale':env['LC_ALL']='C';env.pop('LC_CTYPE')
        if choice=='flags':env['CFLAGS']='-O3'
        if choice=='fd_missing':env.pop('CARGO_MAKEFLAGS')
        if choice=='fd_reversed':env['CARGO_MAKEFLAGS']=f'-j --jobserver-fds={write},{read} --jobserver-auth={write},{read}'
        if choice=='fd_foreign':env['CARGO_MAKEFLAGS']=f'-j --jobserver-fds={read},{other_write} --jobserver-auth={read},{other_write}'
        if choice=='literal':source.write_bytes(b'X'*206)
        if choice=='extent':source.write_bytes(source.read_bytes()+b'X')
        if choice=='hardlink':twin=out/'TEST_CODE_twin';twin.write_bytes(source.read_bytes());source.unlink();os.link(twin,source)
        if choice=='symlink':source.unlink();source.symlink_to(session/'vendor/cc/src/detect_compiler_family.c')
        if choice=='overflow':source=out/'18446744073709551616detect_compiler_family.c';source.write_bytes((session/'vendor/cc/src/detect_compiler_family.c').read_bytes());args=['-E',str(source)]
        if choice=='unknown':root=app
        if choice=='compile':args=['-c',str(header),'-o',str(out/'TEST_CODE.o')]
        if choice=='archive':role='AR';args=['cqD',str(out/'TEST_CODE.a'),str(out/'TEST_CODE.o')]
        if choice=='help':args=['-?']
        if choice=='version_probe':args=['--version']
        if choice=='extra':args=['-E',str(source),'TEST_CODE_extra']
        if choice=='retry':args=['-E','--',str(source)]
    def execute(raw,cwd=root,selected_role=role):
        result=subprocess.run([env[selected_role],*raw],cwd=cwd,env=env,pass_fds=tuple(opened),stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        results.append({'name':name,'args':raw,'status':result.returncode,'stdout_hex':result.stdout.hex(),'stderr_hex':result.stderr.hex()})
        return result
    if CASE=='parallel' and ordinal==2:
        marker=session/('e4-pending-'+name);deadline=time.monotonic()+10
        while not marker.exists():
            assert time.monotonic()<deadline,'TEST_CODE pending window not observed';time.sleep(0.01)
        pending=marker.read_text();pending_call=session/'foreign-native-invocations'/pending
        assert pending_call.is_dir() and not (pending_call/'request.json').exists() and not (pending_call/'receipt.json').exists()
        (session/('e4-observed-'+name)).write_text(json.dumps({'operation_id':pending,'request_absent':True,'receipt_absent':True}))
    saved_owner=(session/'owner.json').read_bytes()
    result=execute(args)
    if CASE=='control_post':(session/'owner.json').write_bytes(saved_owner)
    if CASE=='parallel' and ordinal==2:
        assert result.returncode==0,result.stderr
        receipts=[p for p in (session/'foreign-native-invocations').glob('*/receipt.json')
                  if json.loads(p.read_bytes()).get('cwd_hex')==os.fsencode(str(root)).hex()
                  and json.loads(p.read_bytes()).get('args_hex')==[os.fsencode(v).hex() for v in args]]
        assert len(receipts)==1;release=receipts[0];r=json.loads(release.read_bytes())
        assert r['operation_id']==release.parent.name and r['protocol_state']=='Completed' and r['tool_result']==0 and r['failures']==[]
        (session/('e4-release-'+name)).write_text(json.dumps({'operation_id':release.parent.name,'receipt_sha256':hashlib.sha256(release.read_bytes()).hexdigest()}))
    if CASE=='duplicate':execute(args)
    if CASE.startswith('warning_') and result.returncode==3:execute(['-E','--',str(source)])
    if CASE=='predecessor_tamper':
        # Genuine warning output precedes a label corruption; no second child authority.
        p=next(p for p in (session/'foreign-native-invocations').glob('*/receipt.json') if json.loads(p.read_bytes()).get('context',{}).get('manifest')==str(root))
        r=json.loads(p.read_bytes());r['source_semantics']['effective_stdout']=True;p.write_text(json.dumps(r));execute(['-E','--',str(source)])
    if CASE=='flag_negative' and name=='zstd-sys':
        flag=out/'flag_check.c';flag.write_bytes(b'int main(void) { return 0; }')
        execute(['-O0','-ffunction-sections','-fdata-sections','-fPIC','-m64','-arch','x86_64','-mmacosx-version-min=26.5','-Wall','-Wextra','-fmerge-all-constants','-o',str(out/'flag_check'),'-c',str(flag)],cwd=out,selected_role='CC')
    if CASE not in ('post_missing','post_change') and source.exists() and not source.is_symlink():source.unlink()
if CASE in ('normal','parallel'):
    jobs=[(name,i) for name,facts in E4_FACTS.items() for i in range(1,facts[-1]+1)]
    if CASE=='parallel':
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:list(executor.map(lambda job:family(*job),jobs))
    else:
        for job in jobs:family(*job)
else:
    family('lz4-sys',1);family('zstd-sys',1)
assert [os.fstat(fd).st_ino for fd in (read,write)]==original_stats
for fd in opened:os.close(fd)
(session/'foreign-forwarded.json').write_text(json.dumps(results))
paths=list((session/'foreign-native-invocations').glob('*/receipt.json'))
if CASE=='snapshot_bad':
    for p in paths:(p.parent/'input-post.raw').write_bytes(b'TEST_CODE_changed_snapshot')
if CASE=='stream_bad':
    for p in paths:(p.parent/'stdout.raw').write_bytes(b'TEST_CODE_changed_stdout')
if CASE=='orphan':
    call=session/'foreign-native-invocations'/('f'*32);call.mkdir()
if CASE.startswith('copy_declaration_'):
    kind,index=CASE[len('copy_declaration_'):].rsplit('_',1);name=list(E4_FACTS)[int(index)]
    p=next(p for p in paths if json.loads(p.read_bytes()).get('context',{}).get('manifest')==str(session/'vendor'/name))
    header=session/'vendor'/name/('liblz4/lib/lz4.h' if name=='lz4-sys' else 'zstd/lib/common/allocations.h')
    original=header.read_bytes()
    if kind=='retained':
        body=original;header.chmod(0o644);header.write_bytes(b'TEST_CODE_retained_header_changed:'+name.encode());(p.parent/'request.json').write_bytes(b'{TEST_CODE_bad_request')
        if index=='1':
            r=json.loads(p.read_bytes());members=r['e_only_input_declaration']['source_sha256']
            r['e_only_input_declaration']['source_sha256']={'\x00TEST_CODE_bad_member':'0'*64,**members};p.write_text(json.dumps(r))
    else:
        assert kind=='request_only';header.chmod(0o644);header.write_bytes(b'TEST_CODE_current_header:'+name.encode());body=header.read_bytes();p.unlink()
        for q in p.parent.glob('*.raw'):q.unlink()
    copy=target/'TEST_CODE_copied_header.bin';copy.write_bytes(body)
    (session/'e4-declaration-copy.json').write_text(json.dumps({'copy':str(copy),'header':str(header),'body_hex':body.hex(),'operation_id':p.parent.name,'kind':kind,'name':name,'expected_sha256':hashlib.sha256(original).hexdigest()}))
if CASE in ('stream_'+index+'_'+stream+'_'+kind+'_'+cut for index in ('0','1') for stream in ('stdout','stderr') for kind in ('source','extern','output') for cut in ('current','retained_missing','retained_corrupt','request_only')) or CASE=='flag_negative':
    if CASE=='flag_negative':
        name='zstd-sys';kind='source';cut='current';body=(target/'x86_64-apple-darwin/debug/build/zstd-sys-0123456789abcdef/out/flag_check.c').read_bytes()
        p=next(p for p in paths if json.loads(p.read_bytes())['args_hex'][-1]==os.fsencode(str(target/'x86_64-apple-darwin/debug/build/zstd-sys-0123456789abcdef/out/flag_check.c')).hex())
    else:
        index,stream,kind,cut=CASE[len('stream_'):].split('_',3);name=list(E4_FACTS)[int(index)]
        p=next(p for p in paths if json.loads(p.read_bytes()).get('context',{}).get('manifest')==str(session/'vendor'/name))
        body=(p.parent/(stream+'.raw')).read_bytes()
    retained=hashlib.sha256(body).hexdigest();copy=target/'TEST_CODE_e4_copy.bin';copy.write_bytes(body)
    cc_path=next(p for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_bytes()).get('source')==str(session/'vendor/cc/src/lib.rs'))
    cc=json.loads(cc_path.read_bytes())
    if kind=='source':next(o for o in cc['outputs'] if o['kind']=='dep-info')['dep_info']['paths'].append(str(copy))
    if kind=='extern':cc['externs'].append({'name':'e4_copy','path':str(copy)})
    if kind=='output':cc['declared_outputs'].append({'path':str(copy),'kind':'link'})
    cc_path.write_text(json.dumps(cc))
    if cut.startswith('retained_'):
        for q in (session/'foreign-native-invocations').glob('*/*.raw'):
            if hashlib.sha256(q.read_bytes()).hexdigest()==retained:q.unlink()
        if cut=='retained_missing':(p.parent/'request.json').unlink()
        else:
            assert cut=='retained_corrupt';(p.parent/'request.json').write_bytes(b'{TEST_CODE_bad_request')
    if cut=='request_only':p.unlink()
    (session/'e4-copy-control.json').write_text(json.dumps({'copy':str(copy),'body_hex':body.hex(),'sha256':retained,'operation_id':p.parent.name,'cc_id':cc_path.parent.name,'cc_receipt_sha256':hashlib.sha256(cc_path.read_bytes()).hexdigest(),'kind':kind,'cut':cut,'name':name}))
# E-only fixture does not manufacture an application compile or selected artifact.
emit({'reason':'build-finished','success':True})
'''



E5_ARCHIVE_CARGO = r"""
    env.pop('LC_ALL',None);env['LC_CTYPE']='C.UTF-8';env['ZERO_AR_DATE']='1';role='AR'
    archive=out/('libring_core_0_17_14_.a' if name=='ring' else 'libpsm_s.a')
    first=selected[:16] if name=='ring' else selected
    tail=selected[16:] if name=='ring' else []
    first_paths=[str(out/basename) for _,basename in first];tail_paths=[str(out/basename) for _,basename in tail]
    geometry={'name':name,'out':str(out),'first_bytes':sum(len(os.fsencode(v)) for v in first_paths),
              'first_plus_next_bytes':sum(len(os.fsencode(v)) for v in first_paths+tail_paths[:1]),'tail_bytes':sum(len(os.fsencode(v)) for v in tail_paths)}
    if name=='ring' and CASE!='follow_geometry':assert geometry['first_bytes']<=4000<geometry['first_plus_next_bytes'] and geometry['tail_bytes']<=4000
    gp=session/'followups-geometry.json';gs=json.loads(gp.read_text()) if gp.exists() else [];gs.append(geometry);gp.write_text(json.dumps(gs))
    controls=[]
    def follow(stage,raw):
        before=set((session/'foreign-native-invocations').iterdir());result=execute(raw)
        added=set((session/'foreign-native-invocations').iterdir())-before;assert len(added)==1
        call=added.pop();control={'name':name,'stage':stage,'args':raw,'operation_id':call.name,'status':result.returncode}
        controls.append(control)
        cp=session/'followups-control.json';rows=json.loads(cp.read_text()) if cp.exists() else [];rows.append(control);cp.write_text(json.dumps(rows))
        return control
    d=follow('FirstD',['cqD',str(archive),*first_paths])
    if name=='psm' and CASE=='follow_index_early':
        follow('PSMIndexS',['s',str(archive)])
    else:
        if not (name=='ring' and CASE=='follow_tail_early'):follow('FirstFallbackCQ',['cq',str(archive),*first_paths])
        poison=None
        poison_case=(name=='ring' and CASE in ('follow_tail_prior_raw_out','follow_tail_prior_raw_cwd')) or (name=='psm' and CASE in ('follow_index_prior_raw_out','follow_index_prior_raw_cwd'))
        if poison_case:
            prefix=controls[-1];prefix_path=session/'foreign-native-invocations'/prefix['operation_id']/'receipt.json'
            prefix_receipt=json.loads(prefix_path.read_text());archive_before=archive.read_bytes()
            poison=follow('RefusedPrior',['s' if name=='ring' else 'sD',str(archive)])
            call=session/'foreign-native-invocations'/poison['operation_id'];rp=call/'receipt.json';retained=json.loads(rp.read_text())
            assert retained['protocol_state']=='ProtocolRefused' and retained['tool_result'] is None and retained['context']==prefix_receipt['context']
            qp=call/'request.json';q=json.loads(qp.read_text())
            if CASE.endswith('_raw_out'):q['environment_hex'][os.fsencode('OUT_DIR').hex()]=os.fsencode(str(out.parent/'TEST_CODE_poison_out')).hex()
            else:q['cwd_hex']=os.fsencode(str(root.parent/'TEST_CODE_poison_cwd')).hex()
            qp.write_text(json.dumps(q))
            poison={'name':name,'operation_id':poison['operation_id'],'prefix_id':prefix['operation_id'],
                'archive_path':str(archive),'archive_before_sha256':hashlib.sha256(archive_before).hexdigest(),
                'prefix_receipt_before_sha256':hashlib.sha256(prefix_path.read_bytes()).hexdigest(),
                'ledger_before':prefix_receipt['archive_operand_ledger_return'],
                'poison_receipt_sha256':hashlib.sha256(rp.read_bytes()).hexdigest()}
        if name=='ring':
            raw=['cq',str(archive),*tail_paths]
            if CASE=='follow_tail_order':raw[-1],raw[-2]=raw[-2],raw[-1]
            if CASE=='follow_tail_duplicate_member':raw[-1]=raw[-2]
            if CASE=='follow_tail_readd':raw=['cq',str(archive),*first_paths,*tail_paths]
            if CASE=='follow_tail_d':raw[0]='cqD'
            if CASE=='follow_archive_drift':archive.write_bytes(b'TEST_CODE_unowned_archive_drift')
            if CASE=='follow_archive_recreate':archive.unlink();archive.write_bytes(b'TEST_CODE_recreated_archive')
            if CASE in ('follow_history_missing','follow_history_refused','follow_history_none','follow_history_producer','follow_history_role'):
                c=session/'foreign-native-invocations'/d['operation_id'];rp=c/'receipt.json'
                if CASE=='follow_history_role':
                    qp=c/'request.json';q=json.loads(qp.read_text());q['role']='cc';qp.write_text(json.dumps(q))
                elif CASE=='follow_history_missing':rp.unlink()
                else:
                    r=json.loads(rp.read_text())
                    if CASE=='follow_history_refused':r['protocol_state']='ProtocolRefused';r['failures']=['TEST_CODE_sticky']
                    if CASE=='follow_history_none':r['operation']=None
                    if CASE=='follow_history_producer':r['archive_member_producers_pre']=r['archive_member_producers_pre'][1:]
                    rp.write_text(json.dumps(r))
            follow('RingRemainingCQ',raw)
            if CASE=='follow_tail_repeat':follow('RingRemainingCQ',raw)
        else:
            raw=['s',str(archive)]
            if CASE=='follow_index_member':raw.append(first_paths[0])
            if CASE=='follow_index_d':raw[0]='sD'
            follow('PSMIndexS',raw)
        if poison is not None:
            poison['next_id']=controls[-1]['operation_id'];poison['archive_after_sha256']=hashlib.sha256(archive.read_bytes()).hexdigest()
            poison['prefix_receipt_after_sha256']=hashlib.sha256(prefix_path.read_bytes()).hexdigest()
            poison['ledger_after']=json.loads(prefix_path.read_text())['archive_operand_ledger_return']
            (session/'followups-poison-control.json').write_text(json.dumps(poison))
    role='CC'
"""

E5_NATIVE_AR = r"""
import json,os,pathlib,signal,sys,uuid
args=sys.argv[1:];env=dict(os.environ);session=pathlib.Path(env['E1_SESSION']);case=env['E1_CASE'];name=env['CARGO_PKG_NAME']
pair=tuple(map(int,env['CARGO_MAKEFLAGS'].split('--jobserver-fds=')[1].split()[0].split(',')))
stage='FirstD' if args[0]=='cqD' else 'PSMIndexS' if args[0]=='s' else 'RingRemainingCQ' if name=='ring' and len(args)==15 else 'FirstFallbackCQ'
hits=session/'archive-entry';hits.mkdir(exist_ok=True)
(hits/uuid.uuid4().hex).write_text(json.dumps({'argv':args,'stage':stage,'cwd':str(pathlib.Path.cwd()),'environment':env,
    'fds':list(pair),'inodes':[os.fstat(fd).st_ino for fd in pair],'stdin_eof':sys.stdin.buffer.read()==b''}))
archive=pathlib.Path(args[1]);body=b'TEST_CODE_followup_archive:'+name.encode()+b':'+stage.encode()+b'\n'
if stage!='FirstD' or case=='follow_partial':archive.write_bytes((archive.read_bytes() if archive.exists() else b'')+body)
os.write(1,b'TEST_CODE_followup_stdout\n');os.write(2,b'TEST_CODE_followup_stderr\n')
if stage=='PSMIndexS' and case=='follow_index_input_post':(archive.parent/'4f9a91766097c4c5-x86_64.o').write_bytes(b'TEST_CODE_changed_index_member')
if stage=='PSMIndexS' and case=='follow_index_source_post':
    p=pathlib.Path.cwd()/'src/arch/x86_64.s';p.chmod(0o644);p.write_bytes(b'TEST_CODE_changed_index_source')
if stage=='PSMIndexS' and case=='follow_index_signal':os.kill(os.getpid(),signal.SIGTERM)
sys.exit(7 if stage=='FirstD' and case=='follow_partial' else 1 if stage=='FirstD' else 9 if stage=='RingRemainingCQ' and case=='follow_tail_nonzero' else 11 if stage=='PSMIndexS' and case=='follow_index_nonzero' else 0)
"""


# Fixed RingIndexS protocol fixtures only, never an actual native/provider proof.
E6_RING_INDEX_CARGO = r'''
    if name=='ring':
        prefix=controls[-1];prefix_path=session/'foreign-native-invocations'/prefix['operation_id']/'receipt.json'
        if CASE in ('ring_index_prior_raw_out','ring_index_prior_raw_cwd'):
            poison=follow('RefusedPrior',['sD',str(archive)])
            pc=session/'foreign-native-invocations'/poison['operation_id'];rp=pc/'receipt.json';qp=pc/'request.json'
            retained=json.loads(rp.read_text());assert retained['protocol_state']=='ProtocolRefused' and retained['tool_result'] is None
            q=json.loads(qp.read_text())
            if CASE.endswith('_out'):q['environment_hex'][os.fsencode('OUT_DIR').hex()]=os.fsencode(str(out.parent/'TEST_CODE_wrong_out')).hex()
            else:q['cwd_hex']=os.fsencode(str(root.parent/'TEST_CODE_wrong_cwd')).hex()
            qp.write_text(json.dumps(q))
        if CASE=='ring_index_history_missing':prefix_path.unlink()
        if CASE in ('ring_index_pin_ledger','ring_index_ledger_order'):
            r=json.loads(prefix_path.read_text())
            if CASE=='ring_index_pin_ledger':r['compile_pins_return']={}
            else:r['archive_operand_ledger_return']['producers'].reverse()
            prefix_path.write_text(json.dumps(r))
        raw=['s',str(archive)]
        if CASE=='ring_index_d':raw[0]='sD'
        if CASE=='ring_index_member':raw.append(first_paths[0])
        if CASE=='ring_index_test_archive':raw[1]=str(out/'libring_core_0_17_14__test.a')
        if CASE=='ring_index_archive_alias':raw[1]=str(out/'TEST_CODE_archive_alias'/'..'/archive.name)
        if CASE=='ring_index_member_pre':(out/'25ac62e5b3c53843-curve25519.o').write_bytes(b'TEST_CODE_index_member_pre_changed')
        def index_follow():
            entries=session/'archive-entry';before=set(entries.iterdir())
            archive_before=hashlib.sha256(archive.read_bytes()).hexdigest()
            prior_before=hashlib.sha256(prefix_path.read_bytes()).hexdigest() if prefix_path.exists() else None
            c=follow('RingIndexS',raw)
            c.update(entry_new=sorted(p.name for p in set(entries.iterdir())-before),archive_before=archive_before,
                archive_after=hashlib.sha256(archive.read_bytes()).hexdigest(),prior_before=prior_before,
                prior_after=hashlib.sha256(prefix_path.read_bytes()).hexdigest() if prefix_path.exists() else None)
            cp=session/'ring-index-control.json';rs=json.loads(cp.read_text()) if cp.exists() else [];rs.append(c);cp.write_text(json.dumps(rs))
        index_follow()
        if CASE=='ring_index_repeat':index_follow()
'''


E6_NATIVE = r'''
import json,os,pathlib,signal,sys,uuid
args=sys.argv[1:];env=dict(os.environ);session=pathlib.Path(env['E6_SESSION']);case=env['E6_CASE']
pair=tuple(map(int,env['CARGO_MAKEFLAGS'].split('--jobserver-fds=')[1].split()[0].split(',')))
hits=session/'e6-entry';hits.mkdir(exist_ok=True)
(hits/uuid.uuid4().hex).write_text(json.dumps({'argv':args,'cwd':str(pathlib.Path.cwd()),'environment':env,
    'stdin_eof':sys.stdin.buffer.read()==b'','fds':list(pair),'inodes':[os.fstat(fd).st_ino for fd in pair]}))
if args==['-?']:
    os.write(1,b'TEST_CODE help stdout\n');os.write(2,b'TEST_CODE help rejected\n');sys.exit(0 if case=='help_zero' and env.get('E6_CYCLE')=='2' else 1)
if args==['--version']:
    os.write(1,b'ziglang TEST_CODE\n' if case=='zig' and env.get('E6_CYCLE')=='2' else b'Apple clang TEST_CODE\n');sys.exit(2 if case=='version_nonzero' and env.get('E6_CYCLE')=='2' else 0)
if args[:1]==['-E']:
    if case in ('warning','warning0') and '--' not in args:
        os.write(2,b'-Wslash-u-filename TEST_CODE\n');sys.exit(0 if case=='warning0' else 3)
    os.write(1,b'"clang" "gcc" TEST_CODE family\n');sys.exit(0)
assert args[-2]=='-c' and args[-4]=='-o'
flag=args[9];output=pathlib.Path(args[-3]);source=pathlib.Path(args[-1])
output.write_bytes(b'TEST_CODE flag output:'+flag.encode())
os.write(1,b'TEST_CODE flag stdout:'+flag.encode()+b'\n')
if case=='post_input':source.write_bytes(b'X'*28)
if case=='unsupported' and flag=='-fdata-sections':os.write(2,b'TEST_CODE genuine unsupported warning\n')
if case=='unsupported' and flag=='-fmerge-all-constants':os.kill(os.getpid(),signal.SIGTERM)
sys.exit(0)
'''

E6_CARGO = r'''

import concurrent.futures,hashlib,json,os,pathlib,subprocess,sys,threading,time
CASE=__CASE__;argv=sys.argv[1:];app=pathlib.Path(argv[argv.index('--manifest-path')+1]).parent;session=app.parent
target=session/'target';host=target/'debug/deps';deps=target/'x86_64-apple-darwin/debug/deps'
base=dict(os.environ,E4_SESSION=str(session),E4_CASE=CASE,D1_SESSION=str(session),D1_CASE='normal',CARGO_ENCODED_RUSTFLAGS='')
def emit(e):print(json.dumps(e),flush=True)
def compile(name,source,pkg,dest,extra=()):
    root=source.parent.parent;env=dict(base,CARGO_MANIFEST_DIR=str(root),CARGO_MANIFEST_PATH=str(root/'Cargo.toml'),
        CARGO_PKG_NAME=name,CARGO_PKG_VERSION='1.2.59' if name=='cc' else '0.0.0',
        DYLD_FALLBACK_LIBRARY_PATH=str(host)+':'+os.environ['DYLD_FALLBACK_LIBRARY_PATH'])
    args=['--crate-name',name,'--edition=2021',str(source),'--crate-type','lib','--emit=dep-info,metadata,link','--out-dir',str(dest),*extra]
    result=subprocess.run([env['RUSTC_WRAPPER'],env['RUSTC'],*args],cwd=root,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    assert result.returncode==0,result.stderr
    emit({'reason':'compiler-artifact','package_id':pkg,'target':{'src_path':str(source),'name':name,'kind':['lib'],'crate_types':['lib']},
          'features':[],'filenames':[str(dest/('lib'+name+'.rlib')),str(dest/('lib'+name+'.rmeta'))],'executable':None,'fresh':False})
compile('cc',session/'vendor/cc/src/lib.rs','registry+https://github.com/rust-lang/crates.io-index#cc@1.2.59',host)
read,write=os.pipe();other_read,other_write=os.pipe();opened=[read,write,other_read,other_write]
original_stats=[os.fstat(fd).st_ino for fd in (read,write)]
native_loader=':'.join(map(str,(target/'debug',host,pathlib.Path(os.environ['DYLD_FALLBACK_LIBRARY_PATH']).parent/'lib/rustlib/x86_64-apple-darwin/lib',pathlib.Path(os.environ['DYLD_FALLBACK_LIBRARY_PATH']))))
E4_FACTS=__E4_FACTS__

results=[];cut=None
base.update(E6_SESSION=str(session),E6_CASE=CASE)
E4_FACTS=__E4_FACTS__
def environment(name):
    version,components,features,links,count=E4_FACTS[name];root=session/'vendor'/name
    out=target/'x86_64-apple-darwin/debug/build'/(name+'-0123456789abcdef')/'out';out.mkdir(parents=True,exist_ok=True)
    env=dict(base,CARGO=os.environ.get('CARGO',sys.argv[0]),CARGO_MANIFEST_DIR=str(root),CARGO_MANIFEST_PATH=str(root/'Cargo.toml'),
        CARGO_PKG_NAME=name,CARGO_PKG_VERSION=version,CARGO_PKG_VERSION_MAJOR=components[0],CARGO_PKG_VERSION_MINOR=components[1],
        CARGO_PKG_VERSION_PATCH=components[2],CARGO_PKG_VERSION_PRE=components[3],CARGO_MANIFEST_LINKS=links,
        HOST='x86_64-apple-darwin',TARGET='x86_64-apple-darwin',CARGO_CFG_TARGET_ARCH='x86_64',CARGO_CFG_TARGET_OS='macos',CARGO_CFG_TARGET_ENV='',
        CARGO_CFG_TARGET_ENDIAN='little',CARGO_CFG_TARGET_VENDOR='apple',CARGO_CFG_TARGET_POINTER_WIDTH='64',CARGO_CFG_TARGET_FAMILY='unix',
        CARGO_CFG_TARGET_ABI='',CARGO_CFG_UNIX='',CARGO_CFG_FEATURE=','.join(features),DEBUG='true',OPT_LEVEL='0',PROFILE='debug',
        CARGO_CFG_TARGET_FEATURE='cmpxchg16b,fxsr,sse,sse2,sse3,sse4.1,ssse3',CARGO_CFG_TARGET_HAS_ATOMIC='128,16,32,64,8,ptr',
        CARGO_CFG_DEBUG_ASSERTIONS='',CARGO_CFG_PANIC='unwind',OUT_DIR=str(out),LC_CTYPE='C.UTF-8',DYLD_FALLBACK_LIBRARY_PATH=native_loader,
        CARGO_MAKEFLAGS=f'-j --jobserver-fds={read},{write} --jobserver-auth={read},{write}')
    for k in ('LC_ALL','ZERO_AR_DATE'):env.pop(k,None)
    env.update({'CARGO_FEATURE_'+f.upper().replace('-','_'):'1' for f in features})
    return root,out,env
def entries():return len(list((session/'e6-entry').iterdir())) if (session/'e6-entry').exists() else 0
def execute(args,root,env):
    result=subprocess.run([env['CC'],*args],cwd=root,env=env,pass_fds=tuple(opened),stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    results.append({'name':env['CARGO_PKG_NAME'],'args':args,'status':result.returncode,'stdout_hex':result.stdout.hex(),'stderr_hex':result.stderr.hex()})
    return result
def receipts():return [(p,json.loads(p.read_bytes())) for p in (session/'foreign-native-invocations').glob('*/receipt.json')]
def cycle(name,ordinal,cut_case=False):
    global cut
    root,out,env=environment(name);env['E6_CYCLE']=str(ordinal);source=out/(str(ordinal)+'detect_compiler_family.c')
    source.write_bytes((session/'vendor/cc/src/detect_compiler_family.c').read_bytes())
    result=execute(['-E',str(source)],root,env)
    if CASE in ('warning','warning0'):assert result.returncode==(0 if CASE=='warning0' else 3);result=execute(['-E','--',str(source)],root,env)
    assert result.returncode==0,result.stderr
    if cut_case and CASE=='ambiguous':
        twin=out/'99detect_compiler_family.c';twin.write_bytes(source.read_bytes())
        before=entries();args=['-E',str(twin)];execute(args,root,env)
        cut={'child_before':before,'child_after':entries(),'argv':args};return False
    before=entries();result=execute(['-?'],root,env)
    if cut_case and CASE=='help_zero':
        assert result.returncode==0;before=entries();execute(['--version'],root,env);cut={'child_before':before,'child_after':entries(),'argv':['--version']};return False
    assert result.returncode==1,result.stderr
    if cut_case and CASE=='duplicate_help':
        before=entries();execute(['-?'],root,env);cut={'child_before':before,'child_after':entries(),'argv':['-?']};return False
    if cut_case and CASE=='duplicate_version':
        result=execute(['--version'],root,env);assert result.returncode==0
        before=entries();execute(['--version'],root,env);cut={'child_before':before,'child_after':entries(),'argv':['--version']};return False
    result=execute(['--version'],root,env);assert result.returncode==(2 if cut_case and CASE=='version_nonzero' else 0),result.stderr
    source.unlink();return True
cycle('lz4-sys',1)
if CASE!='missing_base':cycle('zstd-sys',1)
flags=('-ffunction-sections','-fdata-sections','-fmerge-all-constants')
for index,flag in enumerate(flags):
    if not cycle('zstd-sys',index+2,index==0):break
    root,out,env=environment('zstd-sys');env['LC_ALL']='C';env.pop('LC_CTYPE')
    src=out/'flag_check.c'
    if not src.exists():src.write_bytes(b'int main(void) { return 0; }')
    args=['-O0','-ffunction-sections','-fdata-sections','-fPIC','-m64','--target=x86_64-apple-macosx','-mmacosx-version-min=26.5','-Wall','-Wextra',flag,
          '-Wno-unused-command-line-argument','-o',str(out/'flag_check'),'-c',str(src)]
    cwd=out
    if index==0:
        if CASE=='flag_order':args[9]=flags[1]
        if CASE=='dedup':args.pop(9)
        if CASE=='extra':args.insert(0,'TEST_CODE_extra')
        if CASE=='flag_cwd':cwd=root
        if CASE=='flag_locale':env.pop('LC_ALL');env['LC_CTYPE']='C.UTF-8'
        if CASE=='flag_literal':src.write_bytes(b'X'*28)
        if CASE=='output_pre':(out/'flag_check').write_bytes(b'TEST_CODE foreign previous output')
        if CASE in ('missing_receipt','raw_out','role','stream','snapshot'):
            p,r=next((p,r) for p,r in receipts() if r.get('operation',{}).get('class')=='CompilerFamilyHelpProbe' and r.get('context',{}).get('out_dir')==str(out) and r['operation']['previous_version'] is not None)
            if CASE=='missing_receipt':p.unlink()
            if CASE in ('raw_out','role'):
                q=p.parent/'request.json';raw=json.loads(q.read_bytes())
                if CASE=='raw_out':raw['environment_hex'][os.fsencode('OUT_DIR').hex()]=os.fsencode(str(session/'TEST_CODE_other')).hex()
                else:raw['role']='ar'
                q.write_text(json.dumps(raw))
            if CASE=='stream':(p.parent/'stdout.raw').write_bytes(b'TEST_CODE corrupt help stream')
            if CASE=='snapshot':
                eid=r['operation']['predecessor'];ep,er=next((q,rr) for q,rr in receipts() if q.parent.name==eid)
                (ep.parent/'input-post.raw').write_bytes(b'TEST_CODE corrupt E snapshot')
    before=entries();result=execute(args,cwd,env)
    if CASE not in ('normal','unsupported','warning','warning0','operation_none','operation_list') and not CASE.startswith('copy_'):
        cut={'child_before':before,'child_after':entries(),'argv':args}
        if CASE in ('post_input','post_output','capture','forward','return_control','return_fd','return_output'):
            following=out/'900detect_compiler_family.c';following.write_bytes((session/'vendor/cc/src/detect_compiler_family.c').read_bytes())
            probe_env=dict(env,LC_CTYPE='C.UTF-8');probe_env.pop('LC_ALL')
            before_follow=entries();followed=execute(['-E',str(following)],root,probe_env)
            cut.update(next_child_before=before_follow,next_child_after=entries(),next_argv=['-E',str(following)],next_status=followed.returncode)
        break
    assert result.returncode in (0,143),result.stderr
if cut is not None:(session/'e6-cut.json').write_text(json.dumps(cut))
if CASE.startswith('copy_'):
    kind,cut=CASE[len('copy_'):].split('_',1)
    p,r=next((p,r) for p,r in receipts() if r.get('operation',{}).get('class')=='CompilerFlagProbe' and r['operation']['index']==(0 if cut=='retained' else 2))
    body=((p.parent/'input-post.raw').read_bytes() if kind=='source' else (p.parent/'output-post.raw').read_bytes());digest=hashlib.sha256(body).hexdigest()
    copy=target/'TEST_CODE_e6_copy.bin';copy.write_bytes(body)
    cp,cc=next((p,json.loads(p.read_bytes())) for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_bytes()).get('source')==str(session/'vendor/cc/src/lib.rs'))
    if kind=='source':next(o for o in cc['outputs'] if o['kind']=='dep-info')['dep_info']['paths'].append(str(copy))
    if kind=='extern':cc['externs'].append({'name':'e6_copy','path':str(copy)})
    if kind=='output':cc['declared_outputs'].append({'path':str(copy),'kind':'link'})
    cp.write_text(json.dumps(cc))
    if cut=='retained':
        for q in (session/'foreign-native-invocations').glob('*/*.raw'):
            if hashlib.sha256(q.read_bytes()).hexdigest()==digest:q.unlink()
        if kind=='source':pathlib.Path(r['operation']['source']).write_bytes(b'X'*28)
        (p.parent/'request.json').write_bytes(b'{TEST_CODE corrupt request')
    if cut=='request_only':
        p.unlink()
        for q in p.parent.glob('*.raw'):q.unlink()
    (session/'e6-copy.json').write_text(json.dumps({'copy':str(copy),'sha256':digest,'operation_id':p.parent.name,'cc_id':cp.parent.name,'cc_receipt_sha256':hashlib.sha256(cp.read_bytes()).hexdigest()}))
if CASE in ('operation_none','operation_list'):
    selected=[(p,r) for p,r in receipts() if isinstance(r.get('operation'),dict) and r['operation'].get('class')=='CompilerFlagProbe' and r['operation'].get('index')==2]
    assert len(selected)==1;p,r=selected[0];assert (r['protocol_state'],r['tool_result'],r['failures'])==('Completed',0,[])
    snapshot=p.parent/r['output_post']['snapshot'];body=snapshot.read_bytes();digest=hashlib.sha256(body).hexdigest()
    assert digest==r['output_post']['sha256'];copy=target/'TEST_CODE_e6_operation_copy.bin';copy.write_bytes(body)
    selected_cc=[(q,json.loads(q.read_bytes())) for q in (session/'invocations').glob('*/receipt.json') if json.loads(q.read_bytes()).get('source')==str(session/'vendor/cc/src/lib.rs')]
    assert len(selected_cc)==1;cp,cc=selected_cc[0]
    next(o for o in cc['outputs'] if o['kind']=='dep-info')['dep_info']['paths'].append(str(copy));cp.write_text(json.dumps(cc))
    before=entries();r['operation']=None if CASE=='operation_none' else [];p.write_text(json.dumps(r))
    (session/'e6-operation-cut.json').write_text(json.dumps({'operation_id':p.parent.name,'receipt_sha256':hashlib.sha256(p.read_bytes()).hexdigest(),
        'snapshot':str(snapshot),'copy':str(copy),'sha256':digest,'cc_id':cp.parent.name,'cc_receipt_sha256':hashlib.sha256(cp.read_bytes()).hexdigest(),
        'child_before':before,'child_after':entries()}))
assert [os.fstat(fd).st_ino for fd in (read,write)]==original_stats
for fd in opened:os.close(fd)
(session/'e6-forwarded.json').write_text(json.dumps(results))
emit({'reason':'build-finished','success':True})
'''




E7_NATIVE = r'''
import json,os,pathlib,signal,stat,sys,uuid
args=sys.argv[1:];env=dict(os.environ);session=pathlib.Path(env['E6_SESSION']);case=env['E6_CASE']
pair=tuple(map(int,env['CARGO_MAKEFLAGS'].split('--jobserver-fds=')[1].split()[0].split(',')))
hits=session/'e6-entry';hits.mkdir(exist_ok=True)
(hits/uuid.uuid4().hex).write_text(json.dumps({'argv':args,'cwd':str(pathlib.Path.cwd()),'environment':env,
    'stdin_eof':sys.stdin.buffer.read()==b'','fds':list(pair),'inodes':[os.fstat(fd).st_ino for fd in pair], 'regular_fds':[fd for fd in range(3,128) if pathlib.Path('/dev/fd/'+str(fd)).exists() and stat.S_ISREG(os.fstat(fd).st_mode)]}))
if args==['-?']:
    os.write(1,b'TEST_CODE help stdout\n');os.write(2,b'TEST_CODE help rejected\n');sys.exit(0 if case=='help_zero' and env.get('E6_CYCLE')=='2' else 1)
if args==['--version']:
    os.write(1,b'ziglang TEST_CODE\n' if case=='zig' and env.get('E6_CYCLE')=='2' else b'Apple clang TEST_CODE\n');sys.exit(2 if case=='version_nonzero' and env.get('E6_CYCLE')=='2' else 0)
if args[:1]==['-E']:
    if case in ('warning','warning0') and '--' not in args:
        os.write(2,b'-Wslash-u-filename TEST_CODE\n');sys.exit(0 if case=='warning0' else 3)
    os.write(1,b'"clang" "gcc" TEST_CODE family\n');sys.exit(0)
if env.get('CARGO_PKG_NAME')=='lz4-sys' and args[-2:][:1]==['-c']:
    case=env['E7_CASE'];output=pathlib.Path(args[-3]);source=pathlib.Path(args[-1])
    if case!='missing_object':output.write_bytes(b'TEST_CODE lz4 object:'+source.name.encode())
    os.write(1,b'TEST_CODE lz4 stdout:'+source.name.encode()+b'\n');os.write(2,b'TEST_CODE lz4 stderr:'+source.name.encode()+b'\n')
    if case in ('header_post','helper_post'):
        header=pathlib.Path('liblz4/lib/lz4.h' if case=='header_post' else '../cc/src/tool.rs');header.chmod(0o600);header.write_bytes(b'TEST_CODE changed pinned lz4 input/control')
    if case=='nonzero':sys.exit(7)
    if case=='signed':os.kill(os.getpid(),signal.SIGTERM)
    sys.exit(0)
assert args[-2]=='-c' and args[-4]=='-o'
flag=args[9];output=pathlib.Path(args[-3]);source=pathlib.Path(args[-1])
output.write_bytes(b'TEST_CODE flag output:'+flag.encode())
os.write(1,b'TEST_CODE flag stdout:'+flag.encode()+b'\n')
if case=='post_input':source.write_bytes(b'X'*28)
if case=='unsupported' and flag=='-fdata-sections':os.write(2,b'TEST_CODE genuine unsupported warning\n')
if case=='unsupported' and flag=='-fmerge-all-constants':os.kill(os.getpid(),signal.SIGTERM)
sys.exit(0)
'''

E7_CARGO = r'''

import concurrent.futures,hashlib,json,os,pathlib,subprocess,sys,threading,time
CASE="normal";E7_CASE=__E7_CASE__;argv=sys.argv[1:];app=pathlib.Path(argv[argv.index('--manifest-path')+1]).parent;session=app.parent
target=session/'target';host=target/'debug/deps';deps=target/'x86_64-apple-darwin/debug/deps'
base=dict(os.environ,E4_SESSION=str(session),E4_CASE=CASE,D1_SESSION=str(session),D1_CASE='normal',CARGO_ENCODED_RUSTFLAGS='')
def emit(e):print(json.dumps(e),flush=True)
def compile(name,source,pkg,dest,extra=()):
    root=source.parent.parent;env=dict(base,CARGO_MANIFEST_DIR=str(root),CARGO_MANIFEST_PATH=str(root/'Cargo.toml'),
        CARGO_PKG_NAME=name,CARGO_PKG_VERSION='1.2.59' if name=='cc' else '0.0.0',
        DYLD_FALLBACK_LIBRARY_PATH=str(host)+':'+os.environ['DYLD_FALLBACK_LIBRARY_PATH'])
    args=['--crate-name',name,'--edition=2021',str(source),'--crate-type','lib','--emit=dep-info,metadata,link','--out-dir',str(dest),*extra]
    result=subprocess.run([env['RUSTC_WRAPPER'],env['RUSTC'],*args],cwd=root,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    assert result.returncode==0,result.stderr
    emit({'reason':'compiler-artifact','package_id':pkg,'target':{'src_path':str(source),'name':name,'kind':['lib'],'crate_types':['lib']},
          'features':[],'filenames':[str(dest/('lib'+name+'.rlib')),str(dest/('lib'+name+'.rmeta'))],'executable':None,'fresh':False})
compile('cc',session/'vendor/cc/src/lib.rs','registry+https://github.com/rust-lang/crates.io-index#cc@1.2.59',host)
read,write=os.pipe();other_read,other_write=os.pipe();opened=[read,write,other_read,other_write]
original_stats=[os.fstat(fd).st_ino for fd in (read,write)]
native_loader=':'.join(map(str,(target/'debug',host,pathlib.Path(os.environ['DYLD_FALLBACK_LIBRARY_PATH']).parent/'lib/rustlib/x86_64-apple-darwin/lib',pathlib.Path(os.environ['DYLD_FALLBACK_LIBRARY_PATH']))))
E4_FACTS=__E4_FACTS__

results=[];cut=None
base.update(E6_SESSION=str(session),E6_CASE=CASE)
E4_FACTS=__E4_FACTS__
def environment(name):
    version,components,features,links,count=E4_FACTS[name];root=session/'vendor'/name
    out=target/'x86_64-apple-darwin/debug/build'/(name+'-0123456789abcdef')/'out';out.mkdir(parents=True,exist_ok=True)
    env=dict(base,CARGO=os.environ.get('CARGO',sys.argv[0]),CARGO_MANIFEST_DIR=str(root),CARGO_MANIFEST_PATH=str(root/'Cargo.toml'),
        CARGO_PKG_NAME=name,CARGO_PKG_VERSION=version,CARGO_PKG_VERSION_MAJOR=components[0],CARGO_PKG_VERSION_MINOR=components[1],
        CARGO_PKG_VERSION_PATCH=components[2],CARGO_PKG_VERSION_PRE=components[3],CARGO_MANIFEST_LINKS=links,
        HOST='x86_64-apple-darwin',TARGET='x86_64-apple-darwin',CARGO_CFG_TARGET_ARCH='x86_64',CARGO_CFG_TARGET_OS='macos',CARGO_CFG_TARGET_ENV='',
        CARGO_CFG_TARGET_ENDIAN='little',CARGO_CFG_TARGET_VENDOR='apple',CARGO_CFG_TARGET_POINTER_WIDTH='64',CARGO_CFG_TARGET_FAMILY='unix',
        CARGO_CFG_TARGET_ABI='',CARGO_CFG_UNIX='',CARGO_CFG_FEATURE=','.join(features),DEBUG='true',OPT_LEVEL='0',PROFILE='debug',
        CARGO_CFG_TARGET_FEATURE='cmpxchg16b,fxsr,sse,sse2,sse3,sse4.1,ssse3',CARGO_CFG_TARGET_HAS_ATOMIC='128,16,32,64,8,ptr',
        CARGO_CFG_DEBUG_ASSERTIONS='',CARGO_CFG_PANIC='unwind',OUT_DIR=str(out),LC_CTYPE='C.UTF-8',DYLD_FALLBACK_LIBRARY_PATH=native_loader,
        CARGO_MAKEFLAGS=f'-j --jobserver-fds={read},{write} --jobserver-auth={read},{write}')
    for k in ('LC_ALL','ZERO_AR_DATE'):env.pop(k,None)
    env.update({'CARGO_FEATURE_'+f.upper().replace('-','_'):'1' for f in features})
    return root,out,env
def entries():return len(list((session/'e6-entry').iterdir())) if (session/'e6-entry').exists() else 0
def execute(args,root,env):
    result=subprocess.run([env['CC'],*args],cwd=root,env=env,pass_fds=tuple(opened),stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    results.append({'name':env['CARGO_PKG_NAME'],'args':args,'status':result.returncode,'stdout_hex':result.stdout.hex(),'stderr_hex':result.stderr.hex()})
    return result
def receipts():return [(p,json.loads(p.read_bytes())) for p in (session/'foreign-native-invocations').glob('*/receipt.json')]
def cycle(name,ordinal,cut_case=False):
    global cut
    root,out,env=environment(name);env['E6_CYCLE']=str(ordinal);source=out/(str(ordinal)+'detect_compiler_family.c')
    source.write_bytes((session/'vendor/cc/src/detect_compiler_family.c').read_bytes())
    result=execute(['-E',str(source)],root,env)
    if CASE in ('warning','warning0'):assert result.returncode==(0 if CASE=='warning0' else 3);result=execute(['-E','--',str(source)],root,env)
    assert result.returncode==0,result.stderr
    if cut_case and CASE=='ambiguous':
        twin=out/'99detect_compiler_family.c';twin.write_bytes(source.read_bytes())
        before=entries();args=['-E',str(twin)];execute(args,root,env)
        cut={'child_before':before,'child_after':entries(),'argv':args};return False
    before=entries();result=execute(['-?'],root,env)
    if cut_case and CASE=='help_zero':
        assert result.returncode==0;before=entries();execute(['--version'],root,env);cut={'child_before':before,'child_after':entries(),'argv':['--version']};return False
    assert result.returncode==1,result.stderr
    if cut_case and CASE=='duplicate_help':
        before=entries();execute(['-?'],root,env);cut={'child_before':before,'child_after':entries(),'argv':['-?']};return False
    if cut_case and CASE=='duplicate_version':
        result=execute(['--version'],root,env);assert result.returncode==0
        before=entries();execute(['--version'],root,env);cut={'child_before':before,'child_after':entries(),'argv':['--version']};return False
    result=execute(['--version'],root,env);assert result.returncode==(2 if cut_case and CASE=='version_nonzero' else 0),result.stderr
    source.unlink();return True
cycle('lz4-sys',1)
if E7_CASE in ('normal','parallel'):
    if CASE!='missing_base':cycle('zstd-sys',1)
    flags=('-ffunction-sections','-fdata-sections','-fmerge-all-constants')
    for index,flag in enumerate(flags):
        if not cycle('zstd-sys',index+2,index==0):break
        root,out,env=environment('zstd-sys');env['LC_ALL']='C';env.pop('LC_CTYPE')
        src=out/'flag_check.c'
        if not src.exists():src.write_bytes(b'int main(void) { return 0; }')
        args=['-O0','-ffunction-sections','-fdata-sections','-fPIC','-m64','--target=x86_64-apple-macosx','-mmacosx-version-min=26.5','-Wall','-Wextra',flag,
              '-Wno-unused-command-line-argument','-o',str(out/'flag_check'),'-c',str(src)]
        cwd=out
        if index==0:
            if CASE=='flag_order':args[9]=flags[1]
            if CASE=='dedup':args.pop(9)
            if CASE=='extra':args.insert(0,'TEST_CODE_extra')
            if CASE=='flag_cwd':cwd=root
            if CASE=='flag_locale':env.pop('LC_ALL');env['LC_CTYPE']='C.UTF-8'
            if CASE=='flag_literal':src.write_bytes(b'X'*28)
            if CASE=='output_pre':(out/'flag_check').write_bytes(b'TEST_CODE foreign previous output')
            if CASE in ('missing_receipt','raw_out','role','stream','snapshot'):
                p,r=next((p,r) for p,r in receipts() if r.get('operation',{}).get('class')=='CompilerFamilyHelpProbe' and r.get('context',{}).get('out_dir')==str(out) and r['operation']['previous_version'] is not None)
                if CASE=='missing_receipt':p.unlink()
                if CASE in ('raw_out','role'):
                    q=p.parent/'request.json';raw=json.loads(q.read_bytes())
                    if CASE=='raw_out':raw['environment_hex'][os.fsencode('OUT_DIR').hex()]=os.fsencode(str(session/'TEST_CODE_other')).hex()
                    else:raw['role']='ar'
                    q.write_text(json.dumps(raw))
                if CASE=='stream':(p.parent/'stdout.raw').write_bytes(b'TEST_CODE corrupt help stream')
                if CASE=='snapshot':
                    eid=r['operation']['predecessor'];ep,er=next((q,rr) for q,rr in receipts() if q.parent.name==eid)
                    (ep.parent/'input-post.raw').write_bytes(b'TEST_CODE corrupt E snapshot')
        before=entries();result=execute(args,cwd,env)
        if CASE not in ('normal','unsupported','warning','warning0','operation_none','operation_list') and not CASE.startswith('copy_'):
            cut={'child_before':before,'child_after':entries(),'argv':args}
            if CASE in ('post_input','post_output','capture','forward','return_control','return_fd','return_output'):
                following=out/'900detect_compiler_family.c';following.write_bytes((session/'vendor/cc/src/detect_compiler_family.c').read_bytes())
                probe_env=dict(env,LC_CTYPE='C.UTF-8');probe_env.pop('LC_ALL')
                before_follow=entries();followed=execute(['-E',str(following)],root,probe_env)
                cut.update(next_child_before=before_follow,next_child_after=entries(),next_argv=['-E',str(following)],next_status=followed.returncode)
            break
        assert result.returncode in (0,143),result.stderr
if cut is not None:(session/'e6-cut.json').write_text(json.dumps(cut))
if CASE.startswith('copy_'):
    kind,cut=CASE[len('copy_'):].split('_',1)
    p,r=next((p,r) for p,r in receipts() if r.get('operation',{}).get('class')=='CompilerFlagProbe' and r['operation']['index']==(0 if cut=='retained' else 2))
    body=((p.parent/'input-post.raw').read_bytes() if kind=='source' else (p.parent/'output-post.raw').read_bytes());digest=hashlib.sha256(body).hexdigest()
    copy=target/'TEST_CODE_e6_copy.bin';copy.write_bytes(body)
    cp,cc=next((p,json.loads(p.read_bytes())) for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_bytes()).get('source')==str(session/'vendor/cc/src/lib.rs'))
    if kind=='source':next(o for o in cc['outputs'] if o['kind']=='dep-info')['dep_info']['paths'].append(str(copy))
    if kind=='extern':cc['externs'].append({'name':'e6_copy','path':str(copy)})
    if kind=='output':cc['declared_outputs'].append({'path':str(copy),'kind':'link'})
    cp.write_text(json.dumps(cc))
    if cut=='retained':
        for q in (session/'foreign-native-invocations').glob('*/*.raw'):
            if hashlib.sha256(q.read_bytes()).hexdigest()==digest:q.unlink()
        if kind=='source':pathlib.Path(r['operation']['source']).write_bytes(b'X'*28)
        (p.parent/'request.json').write_bytes(b'{TEST_CODE corrupt request')
    if cut=='request_only':
        p.unlink()
        for q in p.parent.glob('*.raw'):q.unlink()
    (session/'e6-copy.json').write_text(json.dumps({'copy':str(copy),'sha256':digest,'operation_id':p.parent.name,'cc_id':cp.parent.name,'cc_receipt_sha256':hashlib.sha256(cp.read_bytes()).hexdigest()}))
if CASE in ('operation_none','operation_list'):
    selected=[(p,r) for p,r in receipts() if isinstance(r.get('operation'),dict) and r['operation'].get('class')=='CompilerFlagProbe' and r['operation'].get('index')==2]
    assert len(selected)==1;p,r=selected[0];assert (r['protocol_state'],r['tool_result'],r['failures'])==('Completed',0,[])
    snapshot=p.parent/r['output_post']['snapshot'];body=snapshot.read_bytes();digest=hashlib.sha256(body).hexdigest()
    assert digest==r['output_post']['sha256'];copy=target/'TEST_CODE_e6_operation_copy.bin';copy.write_bytes(body)
    selected_cc=[(q,json.loads(q.read_bytes())) for q in (session/'invocations').glob('*/receipt.json') if json.loads(q.read_bytes()).get('source')==str(session/'vendor/cc/src/lib.rs')]
    assert len(selected_cc)==1;cp,cc=selected_cc[0]
    next(o for o in cc['outputs'] if o['kind']=='dep-info')['dep_info']['paths'].append(str(copy));cp.write_text(json.dumps(cc))
    before=entries();r['operation']=None if CASE=='operation_none' else [];p.write_text(json.dumps(r))
    (session/'e6-operation-cut.json').write_text(json.dumps({'operation_id':p.parent.name,'receipt_sha256':hashlib.sha256(p.read_bytes()).hexdigest(),
        'snapshot':str(snapshot),'copy':str(copy),'sha256':digest,'cc_id':cp.parent.name,'cc_receipt_sha256':hashlib.sha256(cp.read_bytes()).hexdigest(),
        'child_before':before,'child_after':entries()}))
root,out,env=environment('lz4-sys');env['LC_ALL']='C';env.pop('LC_CTYPE');env['E7_CASE']=E7_CASE;env['NUM_JOBS']='12'
sources=('liblz4/lib/lz4.c','liblz4/lib/lz4frame.c','liblz4/lib/lz4hc.c','liblz4/lib/xxhash.c')
flags=['-O3','-ffunction-sections','-fdata-sections','-fPIC','-g','-gdwarf-2','-fno-omit-frame-pointer','-m64','--target=x86_64-apple-macosx','-mmacosx-version-min=26.5','-Wall','-Wextra']
def object_args(index):return [*flags,'-o',str(out/('efce31824dbf3730-'+pathlib.Path(sources[index]).with_suffix('.o').name)),'-c',sources[index]]
def objects():return [(p,r) for p,r in receipts() if isinstance(r.get('operation'),dict) and r['operation'].get('scope')=='CompileOnly']
def cut_execute(args,local_env=env,cwd=root):
    before=entries();result=execute(args,cwd,local_env)
    cut={'child_before':before,'child_after':entries(),'argv':args,'status':result.returncode}
    selected=[(p,r) for p,r in receipts() if [os.fsdecode(bytes.fromhex(v)) for v in r['args_hex']]==args]
    if E7_CASE=='crash':
        assert result.returncode==19 and selected==[]
        raw=[q for q in (session/'foreign-native-invocations').glob('*/request.json') if json.loads(q.read_bytes())['args_hex']==[os.fsencode(a).hex() for a in args]]
        assert len(raw)==1;cut['operation_id']=raw[0].parent.name
    else:assert len(selected)==1;cut['operation_id']=selected[0][0].parent.name
    next_args=object_args(1);before=entries();following=execute(next_args,root,env)
    cut.update(next_child_before=before,next_child_after=entries(),next_argv=next_args,next_status=following.returncode)
    selected=[(p,r) for p,r in receipts() if [os.fsdecode(bytes.fromhex(v)) for v in r['args_hex']]==next_args]
    assert len(selected)==1;cut['next_operation_id']=selected[0][0].parent.name
    (session/'e7-cut.json').write_text(json.dumps(cut))
if E7_CASE in ('normal','parallel') or E7_CASE.startswith('copy_'):
    if E7_CASE=='parallel':
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            future=pool.submit(execute,object_args(0),root,env)
            deadline=time.monotonic()+15
            while not (session/'e7-pending.json').exists():
                assert time.monotonic()<deadline,'TEST_CODE pending ordinary request';time.sleep(0.01)
            pending=json.loads((session/'e7-pending.json').read_bytes());call=session/'foreign-native-invocations'/pending['operation_id']
            assert (call/'request.json').exists() and not (call/'receipt.json').exists() and list(call.glob('*.raw'))==[]
            # The second real wrapper reaches the owner lock and queues before raw publication.
            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as second_pool:
                second=second_pool.submit(execute,object_args(1),root,env)
                before=entries();deadline=time.monotonic()+15
                while not (session/'e7-waiting').exists():
                    assert time.monotonic()<deadline,'TEST_CODE queued ordinary wrapper';time.sleep(0.01)
                assert entries()==before and not second.done()
                raw=[json.loads(q.read_bytes()) for q in (session/'foreign-native-invocations').glob('*/request.json')]
                assert len([q for q in raw if q['args_hex'][-1]==os.fsencode(sources[1]).hex()])==0
                (session/'e7-release').write_bytes(b'TEST_CODE release');result=future.result(timeout=30);assert result.returncode==0,result.stderr
                result=second.result(timeout=30);assert result.returncode==0,result.stderr
                (session/'e7-serialization.json').write_text(json.dumps({'queued_before':before,'queued_after':entries()}))
        for index in (2,3):result=execute(object_args(index),root,env);assert result.returncode==0,result.stderr
    else:
        for index in range(4):result=execute(object_args(index),root,env);assert result.returncode==0,result.stderr
    # Other ordinary domains remain denied even with genuine E/H/V/flag history.
    zr,zo,ze=environment('zstd-sys');ze['LC_ALL']='C';ze.pop('LC_CTYPE')
    denied=[*flags,'-o',str(zo/'TEST_CODE-denied-zstd.o'),'-c','zstd/lib/common/debug.c']
    before=entries();execute(denied,zr,ze)
    (session/'e7-zstd-cut.json').write_text(json.dumps({'child_before':before,'child_after':entries(),'argv':denied}))
elif E7_CASE.startswith('prior_'):
    result=execute(object_args(0),root,env);assert result.returncode==0,result.stderr
    selected=objects();assert len(selected)==1;p,r=selected[0]
    if E7_CASE=='prior_raw_out':
        q=p.parent/'request.json';raw=json.loads(q.read_bytes());raw['environment_hex'][os.fsencode('OUT_DIR').hex()]=os.fsencode(str(session/'TEST_CODE_other')).hex();q.write_text(json.dumps(raw))
    if E7_CASE=='prior_role':
        q=p.parent/'request.json';raw=json.loads(q.read_bytes());raw['role']='ar';q.write_text(json.dumps(raw))
    if E7_CASE=='prior_stream':(p.parent/'stdout.raw').write_bytes(b'TEST_CODE changed ordinary stream')
    if E7_CASE=='prior_receipt':p.unlink()
    if E7_CASE=='prior_snapshot':(p.parent/r['output_post']['snapshot']).write_bytes(b'TEST_CODE changed retained object')
    if E7_CASE=='prior_output':pathlib.Path(r['operation']['output']).write_bytes(b'TEST_CODE changed live object')
    before=entries();result=execute(object_args(1),root,env)
    selected=[(p,r) for p,r in receipts() if [os.fsdecode(bytes.fromhex(v)) for v in r['args_hex']]==object_args(1)];assert len(selected)==1
    (session/'e7-cut.json').write_text(json.dumps({'child_before':before,'child_after':entries(),'argv':object_args(1),'operation_id':selected[0][0].parent.name,'status':result.returncode}))
else:
    args=object_args(0);local_env=dict(env)
    if E7_CASE=='argv':args.insert(0,'TEST_CODE_extra')
    if E7_CASE=='source':args[-1]='liblz4/lib/TEST_CODE.c'
    if E7_CASE=='locale':local_env.pop('LC_ALL');local_env['LC_CTYPE']='C.UTF-8'
    if E7_CASE=='features':local_env['CARGO_FEATURE_STD']='1'
    if E7_CASE=='jobs':local_env['NUM_JOBS']='11'
    if E7_CASE=='invalid_fd':local_env['CARGO_MAKEFLAGS']='-j --jobserver-fds=999,998 --jobserver-auth=999,998'
    if E7_CASE=='output_alias':pathlib.Path(args[-3]).symlink_to(out/'TEST_CODE aliased output')
    if E7_CASE=='output_pre':pathlib.Path(args[-3]).write_bytes(b'TEST_CODE previous object')
    if E7_CASE in ('header_pre','helper_pre'):
        header=root/('liblz4/lib/lz4.h' if E7_CASE=='header_pre' else '../cc/src/tool.rs');header.chmod(0o600);header.write_bytes(b'TEST_CODE changed pinned lz4 input/control')
    cut_execute(args,local_env)
if E7_CASE.startswith('copy_'):
    kind,cut=E7_CASE[len('copy_'):].split('_',1)
    selected=[(p,r) for p,r in objects() if r['operation']['raw_source']==sources[0]];assert len(selected)==1;p,r=selected[0]
    body=(p.parent/('stdout.raw' if cut=='stream' else r['output_post']['snapshot'])).read_bytes();digest=hashlib.sha256(body).hexdigest()
    copy=target/'TEST_CODE_e7_copy.bin';copy.write_bytes(body)
    selected_cc=[(q,json.loads(q.read_bytes())) for q in (session/'invocations').glob('*/receipt.json') if json.loads(q.read_bytes()).get('source')==str(session/'vendor/cc/src/lib.rs')]
    assert len(selected_cc)==1;cp,cc=selected_cc[0]
    if kind=='source':next(o for o in cc['outputs'] if o['kind']=='dep-info')['dep_info']['paths'].append(str(copy))
    if kind=='extern':cc['externs'].append({'name':'e7_copy','path':str(copy)})
    if kind=='output':cc['declared_outputs'].append({'path':str(copy),'kind':'link'})
    cp.write_text(json.dumps(cc))
    if cut=='retained':
        for q in (session/'foreign-native-invocations').glob('*/*.raw'):
            if hashlib.sha256(q.read_bytes()).hexdigest()==digest:q.unlink()
        pathlib.Path(r['operation']['output']).write_bytes(b'TEST_CODE current differs from retained object')
        (p.parent/'request.json').write_bytes(b'{TEST_CODE malformed raw ordinary request')
    if cut=='request_only':
        p.unlink()
        for q in p.parent.glob('*.raw'):q.unlink()
    (session/'e7-copy.json').write_text(json.dumps({'copy':str(copy),'sha256':digest,'operation_id':p.parent.name,'cc_id':cp.parent.name,'cc_receipt_sha256':hashlib.sha256(cp.read_bytes()).hexdigest()}))
(session/'e7-forwarded.json').write_text(json.dumps(results))
assert [os.fstat(fd).st_ino for fd in (read,write)]==original_stats
for fd in opened:os.close(fd)
(session/'e6-forwarded.json').write_text(json.dumps(results))
emit({'reason':'build-finished','success':True})
'''



E8_AUX_CARGO = "\n    if name=='ring':\n        e8=[];main=out/'libring_core_0_17_14_.a';aux=out/'libring_core_0_17_14__test.a';object_path=out/'a4019cc0736b0423-constant_time_test.o'\n        main_hash=hashlib.sha256(main.read_bytes()).hexdigest();main_prefix=controls[-1]['operation_id']\n        def auxiliary_call(stage,raw):\n            directory=session/('archive-entry' if role=='AR' else 'foreign-entry');directory.mkdir(exist_ok=True)\n            entries=set(directory.iterdir());before=set((session/'foreign-native-invocations').iterdir());result=execute(raw)\n            added=set((session/'foreign-native-invocations').iterdir())-before;assert len(added)==1\n            call=added.pop();row={'stage':stage,'operation_id':call.name,'argv':raw,'status':result.returncode,\n                'child_before':len(entries),'child_after':len(set(directory.iterdir())), 'receipt_sha256':hashlib.sha256((call/'receipt.json').read_bytes()).hexdigest()}\n            e8.append(row);return row\n        env.pop('LC_ALL',None);env.pop('ZERO_AR_DATE',None);env['LC_CTYPE']='C.UTF-8';role='CC'\n        if CASE!='aux_missing_family':\n            second=out/'12detect_compiler_family.c';second.write_bytes((session/'vendor/cc/src/detect_compiler_family.c').read_bytes())\n            e=auxiliary_call('AuxE',['-E',str(second)]);h=auxiliary_call('AuxH',['-?']);v=auxiliary_call('AuxV',['--version']);second.unlink()\n            if CASE=='aux_extra_family':\n                third=out/'13detect_compiler_family.c';third.write_bytes((session/'vendor/cc/src/detect_compiler_family.c').read_bytes())\n                auxiliary_call('ThirdE',['-E',str(third)]);auxiliary_call('ThirdH',['-?']);auxiliary_call('ThirdV',['--version']);third.unlink()\n            if CASE=='aux_missing_receipt':(session/'foreign-native-invocations'/h['operation_id']/'receipt.json').unlink()\n            if CASE=='aux_stream_drift':(session/'foreign-native-invocations'/e['operation_id']/'stdout.raw').write_bytes(b'TEST_CODE Aux stream drift')\n        prefix=session/'foreign-native-invocations'/main_prefix/'receipt.json'\n        if CASE=='aux_main_index_missing':prefix.unlink()\n        if CASE=='aux_main_raw_out':\n            qp=prefix.with_name('request.json');q=json.loads(qp.read_text());q['environment_hex'][os.fsencode('OUT_DIR').hex()]=os.fsencode(str(out.parent/'TEST_CODE_wrong_main_out')).hex();qp.write_text(json.dumps(q))\n        if CASE=='aux_main_family':\n            choices=[p for p in (session/'foreign-native-invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('operation',{}).get('class')=='CompilerObjectCompile' and json.loads(p.read_text()).get('context',{}).get('manifest')==str(root)]\n            assert len(choices)==29\n            for p in choices:\n                r=json.loads(p.read_text());r['family_pre']={};p.write_text(json.dumps(r))\n        raw=[x.replace('$ROOT',str(root)) for x in E2_PREFIX_DATA['ring']]+['-o',str(object_path),'-c',str(root/'crypto/constant_time_test.c')]\n        if CASE=='aux_wrong_object':raw[-3]=str(out/'0000000000000000-constant_time_test.o')\n        if CASE=='aux_wrong_flags':raw[0]='-O3'\n        if CASE=='aux_source_pre':\n            source_aux=root/'crypto/constant_time_test.c';source_aux.chmod(0o644);source_aux.write_bytes(b'TEST_CODE Aux pre drift')\n        env['LC_ALL']='C';env.pop('LC_CTYPE',None)\n        c=auxiliary_call('C1',raw)\n        if CASE=='aux_duplicate_c1':c=auxiliary_call('DuplicateC1',raw)\n        cp=session/'foreign-native-invocations'/c['operation_id']/'receipt.json';cr=json.loads(cp.read_text())\n        env.pop('LC_ALL',None);env['LC_CTYPE']='C.UTF-8';env['ZERO_AR_DATE']='1';role='AR'\n        if cr['protocol_state']!='Completed' or cr.get('tool_result')!=0:\n            auxiliary_call('AfterC1Cut',['cqD',str(aux),str(object_path)])\n        else:\n            if CASE=='aux_existing_archive':aux.write_bytes(b'TEST_CODE unowned initial Aux archive')\n            d=auxiliary_call('AuxFirstD',['cqD',str(aux),str(object_path)])\n            if CASE in ('aux_prior_raw_out','aux_prior_receiptless'):\n                poison=auxiliary_call('RefusedPrior',['r',str(aux),str(object_path)])\n                rp=session/'foreign-native-invocations'/poison['operation_id']/'receipt.json';qp=rp.with_name('request.json')\n                if CASE=='aux_prior_receiptless':rp.unlink()\n                else:\n                    q=json.loads(qp.read_text());q['environment_hex'][os.fsencode('OUT_DIR').hex()]=os.fsencode(str(out.parent/'TEST_CODE_wrong_aux_out')).hex();qp.write_text(json.dumps(q))\n            if CASE=='aux_early_index':auxiliary_call('EarlyIndex',['s',str(aux)])\n            if CASE=='aux_early_sd':\n                env.pop('ZERO_AR_DATE',None);auxiliary_call('EarlyIndexSD',['sD',str(aux)]);env['ZERO_AR_DATE']='1'\n            if d['status']!=0:\n                fallback=['cq',str(aux),str(object_path)]\n                if CASE=='aux_duplicate_member':fallback.append(str(object_path))\n                if CASE=='aux_archive_drift':aux.write_bytes(b'TEST_CODE Aux archive drift')\n                cq=auxiliary_call('AuxFallbackCQ',fallback)\n                if cq['status']==0:auxiliary_call('AuxIndexS',['s',str(aux)])\n            else:\n                env.pop('ZERO_AR_DATE',None)\n                if CASE=='aux_deterministic_wrong_zero':env['ZERO_AR_DATE']='1'\n                auxiliary_call('AuxIndexSD',['sD',str(aux)])\n        closed=[row for row in e8 if (session/'foreign-native-invocations'/row['operation_id']/'receipt.json').exists()]\n        if any(json.loads((session/'foreign-native-invocations'/row['operation_id']/'receipt.json').read_text())['protocol_state']=='ProtocolRefused' for row in closed):\n            auxiliary_call('StickyAfterCut',['cq',str(aux),str(object_path)])\n        assert hashlib.sha256(main.read_bytes()).hexdigest()==main_hash\n        e8_control={'case':CASE,'rows':e8,'main_archive':str(main),'main_sha256':main_hash,'main_index_id':main_prefix,\n                    'main_index_sha256':hashlib.sha256(prefix.read_bytes()).hexdigest() if prefix.exists() else None,\n                    'aux_archive':str(aux),'c1_object':str(object_path)}\n        (session/'e8-control.json').write_text(json.dumps(e8_control))\n"

E8_AUX_CC = "\nif '-c' in args and pathlib.Path(args[-1]).name=='constant_time_test.c':\n    source=pathlib.Path(args[-1]);output=pathlib.Path(args[args.index('-o')+1])\n    os.write(1,b'TEST_CODE Aux C stdout\\n');os.write(2,b'TEST_CODE Aux C stderr\\n');output.write_bytes(b'TEST_CODE Aux object:'+source.name.encode())\n    if case=='aux_source_post':source.chmod(0o644);source.write_bytes(b'TEST_CODE Aux post drift')\n    sys.exit(7 if case=='aux_object_nonzero' else 0)\n"

E8_AUX_AR = "\nif len(args)>=2 and pathlib.Path(args[1]).name=='libring_core_0_17_14__test.a':\n    pair=tuple(map(int,env['CARGO_MAKEFLAGS'].split('--jobserver-fds=')[1].split()[0].split(',')))\n    hits=session/'archive-entry';hits.mkdir(exist_ok=True)\n    (hits/uuid.uuid4().hex).write_text(json.dumps({'argv':args,'cwd':str(pathlib.Path.cwd()),'environment':env,\n        'fds':list(pair),'inodes':[os.fstat(fd).st_ino for fd in pair],'stdin_eof':sys.stdin.buffer.read()==b''}))\n    archive=pathlib.Path(args[1]);code=0 if args[0]!='cqD' or case in ('aux_deterministic','aux_deterministic_wrong_zero') else 7 if case=='aux_partial_fallback' else 1\n    if code==0 or case=='aux_partial_fallback':archive.write_bytes((archive.read_bytes() if archive.exists() else b'')+b'TEST_CODE Aux archive:'+args[0].encode()+b'\\n')\n    os.write(1,b'TEST_CODE Aux AR stdout\\n');os.write(2,b'TEST_CODE Aux AR stderr\\n');sys.exit(code)\n"

E8_AUX_COPY = "\nif CASE.startswith('aux_copy_'):\n    kind,cut,owned=CASE[len('aux_copy_'):].split('_')\n    receipts=[p for p in paths if isinstance(json.loads(p.read_text()).get('operation'),dict)]\n    selected=[p for p in receipts if json.loads(p.read_text())['operation'].get('ring_build')=='AuxiliaryConstantTimeTest'\n        and json.loads(p.read_text())['operation']['class']==('CompilerObjectCompile' if owned=='object' else 'ArchiverIndex')]\n    assert len(selected)==1;p=selected[0];r=json.loads(p.read_text());state=r['output_post' if owned=='object' else 'archive_post']\n    body=(p.parent/state['snapshot']).read_bytes();retained=hashlib.sha256(body).hexdigest();assert state['sha256']==retained\n    copy=target/'copied-Aux.bin';copy.write_bytes(body)\n    cc_path=next(p for p in (session/'invocations').glob('*/receipt.json') if json.loads(p.read_text()).get('source')==str(session/'vendor/cc/src/lib.rs'))\n    rust=json.loads(cc_path.read_text())\n    if kind=='source':next(o for o in rust['outputs'] if o['kind']=='dep-info')['dep_info']['paths'].append(str(copy))\n    elif kind=='extern':rust['externs'].append({'name':'AuxNativeCopy','path':str(copy)})\n    else:rust['declared_outputs'].append({'path':str(copy),'kind':'link'})\n    cc_path.write_text(json.dumps(rust))\n    if cut=='retained':\n        for q in (session/'foreign-native-invocations').glob('*/*.raw'):\n            if hashlib.sha256(q.read_bytes()).hexdigest()==retained:q.unlink()\n        pathlib.Path(state['path']).unlink(missing_ok=True);(p.parent/'request.json').write_bytes(b'{TEST_CODE invalid Aux request')\n    if cut=='requestonly':p.unlink()\n    (session/'e8-copy.json').write_text(json.dumps({'copy':str(copy),'sha256':retained,'operation_id':p.parent.name,\n        'cc_id':cc_path.parent.name,'cc_receipt_sha256':hashlib.sha256(cc_path.read_bytes()).hexdigest(),'kind':kind,'cut':cut,'owned':owned}))\n"

class RecordingProtocolTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="TEST_CODE_replay_owner_")
        self.root = Path(self.temp.name).resolve()
        self.tool = self.root / "tools/replay_build_owner_v1.py"
        self.tool.parent.mkdir()
        shutil.copyfile(TOOL, self.tool)
        self.policy = self.tool.with_name("replay_build_pin_v1_manifest.json")

    def tearDown(self):
        # Fixtures include immutable copied sources; directories remain owned.
        self.temp.cleanup()

    def prepare(self, mode="normal"):
        app = self.root / "origin"
        write(app / "Cargo.toml", '[package]\nname="stock_analysis"\nversion="0.0.0"\n')
        write(app / "Cargo.lock", "TEST_CODE_FIXED_LOCK\n")
        write(app / "build.rs", "fn main() {}\n")
        write(app / "src/lib.rs", "pub fn fixture() {}\n")
        vendor = self.root / "vendor-origin"
        write(vendor / "dep/Cargo.toml", '[package]\nname="dep"\nversion="0.0.0"\n')
        write(vendor / "dep/src/lib.rs", "pub fn dep() {}\n")
        write(vendor / "dep/.cargo-checksum.json", '{"files":{},"package":"TEST_CODE"}')
        sysroot = self.root / "sysroot"
        write(sysroot / "lib/test.bin", "TEST_CODE_SYSROOT")
        if mode.startswith(("nested:", "autocfg:", "fix5:")):
            write(sysroot / "lib/rustlib/x86_64-apple-darwin/lib/test.bin", "TEST_CODE_HOST_SYSROOT")
            for name, version in (("libc", "0.2.184"), ("proc-macro2", "1.0.106")):
                write(vendor / name / "Cargo.toml", '[package]\nname="' + name + '"\nversion="' + version + '"\n')
                write(vendor / name / "build.rs", "// TEST_CODE generator simulation for " + name + "\n")
                for probe in ("proc_macro_span", "proc_macro_span_file", "proc_macro_span_location"):
                    write(vendor / name / ("src/probe/" + probe + ".rs"), "// TEST_CODE " + probe + "\n")

        if mode.startswith("autocfg:"):
            for name, version in (("num-traits", "0.2.19"), ("autocfg", "1.5.0")):
                write(vendor / name / "Cargo.toml", '[package]\nname="' + name + '"\nversion="' + version + '"\n')
                write(vendor / name / "build.rs", "// TEST_CODE autocfg generator\n")
                for source in ("lib.rs", "rustc.rs", "version.rs"):
                    write(vendor / name / "src" / source, "// TEST_CODE " + name + " " + source + "\n")
        if mode.startswith("fix5:"):
            for name, version in (("rustix", "1.1.4"), ("system-configuration-sys", "0.6.0")):
                write(vendor / name / "Cargo.toml", '[package]\nname="' + name + '"\nversion="' + version + '"\n')
                write(vendor / name / "build.rs", "// TEST_CODE fixed5 generator\n")
                write(vendor / name / "src/lib.rs", "pub fn fixture() {}\n")
        rustc = write(self.root / "fake-rustc", "#!" + PYTHON + " -I\n" + FAKE_RUSTC.replace("__AUTOCFG_BUILDER__", repr(AUTOCFG_BUILDER)).replace("__RUSTIX_BUILDER__", repr(RUSTIX_BUILDER)))
        cargo_source = FAKE_CARGO.replace("compile('dep',session/", NESTED_CARGO_FLOW + AUTOCFG_CARGO_FLOW + FIX5_CARGO_FLOW + "\ncompile('dep',session/", 1)
        cargo = write(self.root / "fake-cargo", "#!" + PYTHON + " -I\n" + cargo_source.replace("__MODE__", repr(mode)).replace("__LINTS__", repr(OBSERVED_LINTS)).replace("__ARG_CASES__", repr(LINT_MUTATIONS)).replace("__WRITEABLE_LINTS__", repr(WRITEABLE_LINTS)))
        rustc.chmod(0o700); cargo.chmod(0o700)
        def pin(path): return {"path": str(path), "sha256": sha(path)}
        inventory = {"owner_sha256": sha(self.tool), "cargo": pin(cargo), "rustc": pin(rustc),
                     "python": pin(Path(PYTHON)), "sysroot": snapshot(sysroot, ["lib"]),
                     "application": snapshot(app, ["Cargo.toml", "Cargo.lock", "build.rs", "src"]),
                     "vendor": snapshot(vendor, ["dep"]), "packages": [
                         {"id": "TEST_CODE_app", "tree": "application", "manifest": "Cargo.toml"},
                         {"id": "TEST_CODE_dep", "tree": "vendor", "manifest": "dep/Cargo.toml"}],
                     "generators": {"PROTOC": pin(rustc)},
                     "environment": {"PATH": str(Path(PYTHON).parent), "PROTOC": str(rustc)},
                     "ancestor_configs": []}
        if mode.startswith("nested:"):
            inventory["vendor"] = snapshot(vendor, ["dep", "libc", "proc-macro2"])
            inventory["packages"].extend({"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
                                           "tree": "vendor", "manifest": name + "/Cargo.toml"}
                                          for name, version in (("libc", "0.2.184"), ("proc-macro2", "1.0.106")))
        if mode.startswith("autocfg:"):
            inventory["vendor"] = snapshot(vendor, ["dep", "num-traits", "autocfg"])
            inventory["packages"].extend({"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
                                           "tree": "vendor", "manifest": name + "/Cargo.toml"}
                                          for name, version in (("num-traits", "0.2.19"), ("autocfg", "1.5.0")))
        if mode.startswith("fix5:"):
            inventory["vendor"] = snapshot(vendor, ["dep", "rustix", "system-configuration-sys"])
            inventory["packages"].extend({"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
                                           "tree": "vendor", "manifest": name + "/Cargo.toml"}
                                          for name, version in (("rustix", "1.1.4"), ("system-configuration-sys", "0.6.0")))
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly", "profile": owner.PROFILE, "inventory": inventory}))
        return inventory

    def invoke(self, *args, incoming=None):
        env = dict(os.environ)
        env.update(incoming or {})
        return subprocess.run([PYTHON, "-I", str(self.tool), *args], env=env,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)

    def record_result(self, result):
        self.assertTrue(result.stdout, result.stderr.decode(errors="replace"))
        reply = json.loads(result.stdout)
        self.assertEqual(reply["state"], "RecordingOnly")
        self.assertNotIn("qualified", reply)
        return json.loads(Path(reply["record_path"]).read_text())

    def test_missing_inventory_creates_no_build_session(self):
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly",
                                           "profile": owner.PROFILE, "inventory": None}))
        result = self.invoke("record")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(json.loads(result.stdout)["reason"], "MissingInventory")
        self.assertFalse((self.root / ".replay-build-records").exists())

    def test_cli_never_accepts_policy_path_qualify_or_shell_tail(self):
        for args in [("record", "--policy", "fake.json"), ("qualify",), ("record", "; echo unsafe")]:
            self.assertEqual(self.invoke(*args).returncode, 2)
        self.assertFalse((self.root / ".replay-build-records").exists())

    def test_full_record_observes_extern_and_actual_generator_without_pin(self):
        inventory = self.prepare()
        result = self.invoke("record", incoming={"RUSTFLAGS": "--sysroot /bad", "PROTOC": "/bad", "CARGO_HOME": "/bad",
                                                "DYLD_FALLBACK_LIBRARY_PATH": "/TEST_CODE_CALLER", "LD_LIBRARY_PATH": "/TEST_CODE_CALLER",
                                                "CARGO_MAKEFLAGS": "TEST_CODE_AMBIENT", "MAKEFLAGS": "TEST_CODE_AMBIENT",
                                                "MFLAGS": "TEST_CODE_AMBIENT"})
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        record = self.record_result(result)
        self.assertEqual(record["blockers"], [])
        self.assertEqual(record["review_gate"], "IndependentPolicyReviewRequired")
        record_path = Path(json.loads(result.stdout)["record_path"])
        actual_owner = json.loads((record_path.parent / "owner.json").read_text())
        self.assertNotIn("RUSTFLAGS", actual_owner["environment"])
        self.assertEqual(actual_owner["environment"]["PROTOC"], inventory["generators"]["PROTOC"]["path"])
        self.assertNotEqual(actual_owner["environment"]["CARGO_HOME"], "/bad")
        self.assertNotIn("LD_LIBRARY_PATH", actual_owner["environment"])
        sysroot_lib = str(Path(inventory["sysroot"]["root"]) / "lib")
        self.assertEqual(actual_owner["environment"]["DYLD_FALLBACK_LIBRARY_PATH"], sysroot_lib)
        session = record_path.parent
        self.assertEqual({p.name for p in (session / "compiler-entry").iterdir()},
                         {"probe", "compile-dep", "compile-build_script_build", "compile-stock_analysis"})
        receipts = [json.loads(p.read_text()) for p in (session / "invocations").glob("*/receipt.json")]
        self.assertEqual(sum(r["kind"] == "Probe" for r in receipts), 1)
        self.assertEqual(sum(r["kind"] == "Compile" for r in receipts), 3)
        for receipt in receipts:
            env = {bytes.fromhex(k).decode(): bytes.fromhex(v).decode() for k, v in receipt["environment_hex"].items()}
            expected = sysroot_lib if receipt["kind"] == "Probe" else str(session / "target/debug/deps") + ":" + sysroot_lib
            self.assertEqual(env["DYLD_FALLBACK_LIBRARY_PATH"], expected)
        self.assertEqual(len(record["selected_library"]), 1)
        self.assertEqual(len(record["extern_edges"]), 1)
        self.assertEqual(len(record["extern_edges"][0]["producers"]), 1)
        self.assertEqual(len(record["build_script_associations"]), 1)
        self.assertTrue(any("generated_by" in item["owner"] for item in record["consumed_sources"]))
        self.assertEqual(sha(Path(inventory["application"]["root"]) / "Cargo.lock"), inventory["application"]["files"]["Cargo.lock"])
        self.assertFalse((self.root / "release").exists())

    def rejected_loader_record(self, mode, expected_env, *, probe_rejected=False):
        inventory = self.prepare(mode)
        result = self.invoke("record")
        self.assertEqual(result.returncode, 2)
        record = self.record_result(result)
        session = Path(json.loads(result.stdout)["record_path"]).parent
        self.assertIn(b"CompilerEnvironmentInjection", (session / "cargo.stderr.raw").read_bytes())
        self.assertIn("CargoDidNotFinishSuccessfully", record["blockers"])
        self.assertTrue(any(b.startswith("IncompleteInvocation:") for b in record["blockers"]))
        self.assertEqual(record["selected_library"], [])
        hit_root = session / "compiler-entry"
        hits = {p.name for p in hit_root.iterdir()} if hit_root.exists() else set()
        self.assertEqual(hits, set() if probe_rejected else {"probe"})
        self.assertFalse(any((session / "target").rglob("*.rlib")))
        calls = list((session / "invocations").iterdir())
        rejected = [call for call in calls if not (call / "receipt.json").exists()]
        self.assertEqual(len(rejected), 1)
        self.assertFalse((rejected[0] / "invocation.json").exists())
        request = json.loads((rejected[0] / "request.json").read_text())
        env = {bytes.fromhex(k).decode(): bytes.fromhex(v).decode() for k, v in request["environment_hex"].items()}
        expected = expected_env(session, str(Path(inventory["sysroot"]["root"]) / "lib"))
        for key, value in expected.items():
            if value is None:
                self.assertNotIn(key, env)
            else:
                self.assertEqual(env[key], value)
        receipts = [json.loads((c / "receipt.json").read_text()) for c in calls if (c / "receipt.json").exists()]
        self.assertTrue(all(r["kind"] == "Probe" and r["outputs"] == [] for r in receipts))

    def test_loader_mutations_refuse_before_compile_entry(self):
        cases = {
            "loader_extra": lambda s, lib: str(s / "target/debug/deps") + ":" + lib + ":/usr/local/lib",
            "loader_reversed": lambda s, lib: lib + ":" + str(s / "target/debug/deps"),
            "loader_foreign": lambda s, lib: str(s.parent / "pending-foreign/target/debug/deps") + ":" + lib,
            "loader_empty": lambda s, lib: "",
            "loader_missing": lambda s, lib: None,
            "loader_empty_component": lambda s, lib: str(s / "target/debug/deps") + "::" + lib,
            "loader_duplicate": lambda s, lib: str(s / "target/debug/deps") + ":" + str(s / "target/debug/deps") + ":" + lib,
        }
        for mode, value in cases.items():
            with self.subTest(mode=mode):
                self.rejected_loader_record(mode, lambda s, lib: {"DYLD_FALLBACK_LIBRARY_PATH": value(s, lib)})

    def test_other_loader_flags_refuse_before_compile_entry(self):
        for key in ("DYLD_LIBRARY_PATH", "LD_PRELOAD", "LD_LIBRARY_PATH"):
            with self.subTest(key=key):
                self.rejected_loader_record("forbidden_" + key, lambda s, lib: {key: "/TEST_CODE_FOREIGN"})
        # Loading DYLD_INSERT_LIBRARIES before Python starts is outside the
        # wrapper TCB; check this guard without asking dyld to load a fake dylib.
        inventory = self.prepare()
        with self.assertRaises(owner.Refusal):
            owner.compiler_environment({"DYLD_INSERT_LIBRARIES": "/TEST_CODE_FOREIGN",
                                        "DYLD_FALLBACK_LIBRARY_PATH": str(Path(inventory["sysroot"]["root"]) / "lib")},
                                       self.root, inventory["sysroot"], probe=True)

    def test_probe_rejects_compile_loader_path_before_compiler_entry(self):
        self.rejected_loader_record("probe_compile_path", lambda s, lib: {
            "DYLD_FALLBACK_LIBRARY_PATH": str(s / "target/debug/deps") + ":" + lib}, probe_rejected=True)

    def test_loader_requires_complete_lib_inventory(self):
        self.prepare()
        policy = json.loads(self.policy.read_text())
        policy["inventory"]["sysroot"]["roots"] = ["lib/test.bin"]
        self.policy.write_text(json.dumps(policy))
        result = self.invoke("record")
        self.assertEqual(result.returncode, 2)
        self.assertIn(b"IncompleteLoaderInventory", result.stderr)
        self.assertFalse((self.root / ".replay-build-records").exists())

    def test_inventory_extras_and_symlinks_are_refused(self):
        inventory = self.prepare()
        extra = Path(inventory["vendor"]["root"]) / "dep/extra"
        extra.write_text("unexpected")
        self.assertEqual(self.invoke("record").returncode, 2)
        extra.unlink()
        extra.symlink_to("src/lib.rs")
        self.assertEqual(self.invoke("record").returncode, 2)
        self.assertFalse((self.root / ".replay-build-records").exists())

    def test_final_sysroot_mismatch_refuses_before_compile(self):
        self.prepare("wrong_sysroot")
        result = self.invoke("record")
        self.assertEqual(result.returncode, 2)
        record = self.record_result(result)
        self.assertIn("CargoDidNotFinishSuccessfully", record["blockers"])
        self.assertEqual(record["selected_library"], [])

    def test_output_written_before_compiler_failure_is_not_success_receipt(self):
        self.prepare("compiler_fails")
        result = self.invoke("record")
        self.assertEqual(result.returncode, 2)
        record = self.record_result(result)
        self.assertIn("CompilerFailed", record["blockers"])
        self.assertEqual(record["selected_library"], [])

    def test_missing_generator_event_never_guesses_producer(self):
        self.prepare("missing_producer")
        result = self.invoke("record")
        self.assertEqual(result.returncode, 2)
        record = self.record_result(result)
        self.assertTrue(any(b.startswith("UnresolvedConsumedSource:") for b in record["blockers"]))
        self.assertEqual(record["build_script_associations"], [])

    def test_output_drift_blocks_clean_diagnostic_record(self):
        self.prepare("output_drift")
        result = self.invoke("record")
        self.assertEqual(result.returncode, 2)
        self.assertTrue(any(b.startswith("ChangedOutput:") for b in self.record_result(result)["blockers"]))

    def test_source_drift_prevents_final_record_seal(self):
        self.prepare("source_drift")
        result = self.invoke("record")
        self.assertEqual(result.returncode, 2)
        self.assertIn(b"InventoryMismatch", result.stderr)
        self.assertFalse(list((self.root / ".replay-build-records").glob("*/record.json")))

    def test_unknown_cargo_event_retains_raw_bytes_and_review_blocker(self):
        self.prepare("unknown_event")
        result = self.invoke("record")
        self.assertEqual(result.returncode, 2)
        self.assertTrue(any(b.startswith("UnknownCargoEvent:") for b in self.record_result(result)["blockers"]))
        raw = next((self.root / ".replay-build-records").glob("*/cargo.stdout.raw")).read_bytes()
        self.assertIn(b"TEST_CODE_unknown", raw)

    def rejected_protocol_record(self, mode):
        self.prepare(mode)
        result = self.invoke("record")
        self.assertEqual(result.returncode, 2, result.stderr.decode(errors="replace"))
        record = self.record_result(result)
        session = Path(json.loads(result.stdout)["record_path"]).parent
        self.assertIn("CargoDidNotFinishSuccessfully", record["blockers"])
        self.assertTrue(any(b.startswith("IncompleteInvocation:") for b in record["blockers"]))
        self.assertEqual(record["selected_library"], [])
        self.assertEqual({p.name for p in (session / "compiler-entry").iterdir()}, {"probe"})
        self.assertFalse(any((session / "target").rglob("*.rlib")))
        rejected = [p for p in (session / "invocations").iterdir() if not (p / "receipt.json").exists()]
        self.assertEqual(len(rejected), 1)
        self.assertFalse((rejected[0] / "invocation.json").exists())
        request = json.loads((rejected[0] / "request.json").read_text())
        return session, request

    def test_equals_lints_preserve_raw_argv_and_record_success(self):
        self.prepare("lint_equals")
        result = self.invoke("record")
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        record = self.record_result(result)
        self.assertEqual(record["blockers"], [])
        self.assertEqual(len(record["selected_library"]), 1)
        session = Path(json.loads(result.stdout)["record_path"]).parent
        count = 0
        for path in (session / "invocations").glob("*/receipt.json"):
            receipt = json.loads(path.read_text())
            if receipt["kind"] == "Probe":
                continue
            name = receipt["parsed"]["options"]["--crate-name"][0]
            argv = [bytes.fromhex(v).decode() for v in receipt["argv_hex"]]
            tokens = [v for v in argv if v.startswith(("--allow=", "--warn="))]
            self.assertEqual(tokens, OBSERVED_LINTS[name])
            self.assertEqual(receipt["exit_code"], 0)
            self.assertEqual(receipt["role"], "Target" if name == "stock_analysis" else "Host")
            observed = json.loads((path.parent / "stderr.raw").read_text())["fixture_argv"]
            self.assertEqual(observed, argv[1:])
            for prefix, key in (("--allow=", "-A"), ("--warn=", "-W")):
                self.assertEqual(receipt["parsed"]["options"].get(key, []),
                                 [v[len(prefix):] for v in tokens if v.startswith(prefix)])
            count += len(tokens)
        self.assertEqual(count, 56)
        self.assertEqual({p.name for p in (session / "compiler-entry").iterdir()},
                         {"probe", "compile-dep", "compile-build_script_build", "compile-stock_analysis"})
        parsed = owner.parse_rustc(["--crate-name=x", "--emit=dep-info", "--out-dir=/tmp/out",
                                    "--allow=unused", "--warn=unused", "--allow=unused", "src/lib.rs"])
        self.assertEqual(parsed["options"]["-A"], ["unused", "unused"])
        self.assertEqual(parsed["options"]["-W"], ["unused"])

    def test_equals_lint_mutations_refuse_before_compiler_entry(self):
        for case, tokens in LINT_MUTATIONS.items():
            with self.subTest(case=case):
                session, request = self.rejected_protocol_record("lint_invalid:" + case)
                argv = [bytes.fromhex(v).decode() for v in request["argv_hex"]]
                self.assertEqual(argv[1:1 + len(tokens)], tokens)
                self.assertIn(b"Refused", (session / "cargo.stderr.raw").read_bytes())

    def test_jobserver_pipe_survives_wrapper_and_unrelated_fd_closes(self):
        self.prepare("jobserver_valid")
        result = self.invoke("record")
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        record = self.record_result(result)
        self.assertEqual(record["blockers"], [])
        self.assertEqual(len(record["selected_library"]), 1)
        session = Path(json.loads(result.stdout)["record_path"]).parent
        seen = set()
        for path in (session / "invocations").glob("*/receipt.json"):
            receipt = json.loads(path.read_text())
            env = {bytes.fromhex(k).decode(): bytes.fromhex(v).decode()
                   for k, v in receipt["environment_hex"].items()}
            if receipt["kind"] == "Probe":
                self.assertNotIn("CARGO_MAKEFLAGS", env)
                continue
            name = receipt["parsed"]["options"]["--crate-name"][0]
            attempt = json.loads((session / ("jobserver-attempt-" + name + ".json")).read_text())
            observed = json.loads((session / ("jobserver-observed-" + name + ".json")).read_text())
            self.assertEqual(observed["pair"], attempt["pair"])
            self.assertEqual(observed["canary"], attempt["canary"])
            self.assertTrue(observed["canary_closed"])
            self.assertEqual(observed["token_hex"], "4a")
            self.assertEqual(env["CARGO_MAKEFLAGS"], attempt["environment"]["CARGO_MAKEFLAGS"])
            self.assertEqual(receipt["exit_code"], 0)
            seen.add(name)
        self.assertEqual(seen, {"dep", "build_script_build", "stock_analysis"})
        self.assertEqual({p.name for p in (session / "compiler-entry").iterdir()},
                         {"probe", "compile-dep", "compile-build_script_build", "compile-stock_analysis"})

    def test_jobserver_mutations_refuse_before_compiler_entry(self):
        for case in JOBSERVER_MUTATIONS:
            with self.subTest(case=case):
                session, request = self.rejected_protocol_record("jobserver_invalid:" + case)
                env = {bytes.fromhex(k).decode(): bytes.fromhex(v).decode()
                       for k, v in request["environment_hex"].items()}
                attempt = json.loads((session / "jobserver-attempt-dep.json").read_text())
                self.assertEqual({k: env[k] for k in ("CARGO_MAKEFLAGS", "MAKEFLAGS", "MFLAGS") if k in env},
                                 attempt["environment"])
                self.assertIn(b"Jobserver", (session / "cargo.stderr.raw").read_bytes())
                self.assertFalse(list(session.glob("jobserver-observed-*.json")))

    def test_record_owner_digest_binds_receipt_and_source_distinctly(self):
        inventory = self.prepare("jobserver_missing")
        result = self.invoke("record")
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        record = self.record_result(result)
        self.assertEqual(record["blockers"], [])
        session = Path(json.loads(result.stdout)["record_path"]).parent
        receipt = json.loads((session / "owner.json").read_text())
        self.assertEqual(record["owner_sha256"], sha(session / "owner.json"))
        self.assertEqual(receipt["owner_sha256"], sha(self.tool))
        self.assertEqual(receipt["owner_sha256"], inventory["owner_sha256"])
        self.assertNotEqual(record["owner_sha256"], receipt["owner_sha256"])
        for path in (session / "invocations").glob("*/receipt.json"):
            invocation = json.loads(path.read_text())
            env = {bytes.fromhex(k).decode(): bytes.fromhex(v).decode()
                   for k, v in invocation["environment_hex"].items()}
            self.assertNotIn("CARGO_MAKEFLAGS", env)
            self.assertEqual(invocation["exit_code"], 0)

    def nested_result(self, case="normal", expected=0):
        self.prepare("nested:" + case)
        result = self.invoke("record")
        self.assertEqual(result.returncode, expected, result.stderr.decode(errors="replace"))
        record = self.record_result(result)
        session = Path(json.loads(result.stdout)["record_path"]).parent
        receipts = [(p, json.loads(p.read_text())) for p in (session / "invocations").glob("*/receipt.json")]
        return session, record, receipts

    def test_nested_libc_version_host_and_target_outdir(self):
        session, record, receipts = self.nested_result()
        versions = [r for _, r in receipts if r["context"]["kind"] == "LibcBuildVersion"]
        self.assertEqual(len(versions), 2)
        self.assertEqual({r["context"]["out_dir"] for r in versions}, {
            str(session / "target/debug/build/TEST_CODE_output_libc/out"),
            str(session / "target/x86_64-apple-darwin/debug/build/TEST_CODE_output_libc/out")})
        for r in versions:
            self.assertEqual([bytes.fromhex(x).decode() for x in r["argv_hex"]][1:], ["--version"])
            self.assertEqual(r["exit_code"], 0)
        self.assertEqual(record["blockers"], [])
        _, failure, bad = self.nested_result("version_failure", 2)
        self.assertIn("CompilerFailed", failure["blockers"])
        self.assertTrue(any(r["context"]["kind"] == "LibcBuildVersion" and r["exit_code"] == 1 for _, r in bad))

    def test_nested_context_and_loader_injection_zero_compiler_hits(self):
        first = ("extra", "permuted", "foreign", "empty", "empty_component", "host", "target",
                 "package", "manifest", "wrapper", "outdir", "cwd", "encoded", "bootstrap", "direct_disguise")
        for case in first + ("unknown_probe", "source_substitution", "unknown_flag"):
            with self.subTest(case=case):
                session, record, _ = self.nested_result(case, 2)
                self.assertTrue(any(b.startswith("IncompleteInvocation:") for b in record["blockers"]))
                hits = {p.name for p in (session / "compiler-entry").iterdir()}
                if case in first:
                    self.assertFalse(any(h.startswith("nested-") for h in hits), hits)
                else:
                    self.assertFalse(any(h.startswith("nested-proc_") for h in hits), hits)
                rejected = [p for p in (session / "invocations").iterdir() if not (p / "receipt.json").exists()]
                self.assertEqual(len(rejected), 1)
                self.assertTrue((rejected[0] / "request.json").is_file())
                self.assertFalse((rejected[0] / "invocation.json").exists())
                self.assertEqual(record["selected_library"], [])

    def test_proc_macro_feature_probe_transient_success_and_unsupported(self):
        session, record, receipts = self.nested_result()
        probes = [(p, r) for p, r in receipts if r["kind"] == "TransientProbe"]
        self.assertEqual(len(probes), 3)
        self.assertEqual(sorted(r["exit_code"] for _, r in probes), [0, 0, 1])
        self.assertEqual((session / "nested-status.raw").read_text().splitlines(), ["0", "0", "0", "0", "1"])
        self.assertEqual(len(record["nested_origins"]), 5)
        self.assertEqual(record["blockers"], [])
        seen = set()
        for path, receipt in probes:
            self.assertEqual(receipt["probe_outcome"], "Supported" if receipt["exit_code"] == 0 else "Unsupported")
            self.assertEqual(len(receipt["outputs"]), 2 if receipt["exit_code"] == 0 else 1)
            raw = json.loads((path.parent / "stderr.raw").read_text())
            self.assertEqual(raw["fixture_argv"], [bytes.fromhex(v).decode() for v in receipt["argv_hex"]][1:])
            for output in receipt["outputs"]:
                self.assertFalse(Path(output["path"]).exists())
                snapshot_file = path.parent / output["snapshot"]
                self.assertEqual(sha(snapshot_file), output["sha256"])
                self.assertNotIn(snapshot_file, seen); seen.add(snapshot_file)
            self.assertFalse(any(a["producer_invocation"] == path.parent.name for a in record["build_script_associations"]))
        self.assertEqual(len(seen), 5)
        self.assertEqual(record["extern_edges"], [])

    def test_transient_probe_evidence_failure_is_blocker(self):
        for case, blocker in (("snapshot_tamper", "ChangedTransientEvidence:"),
                              ("snapshot_missing", "ChangedTransientEvidence:"),
                              ("exit2", "CompilerFailed"), ("signal", "CompilerFailed"),
                              ("symlink", "TransientEvidence:"),
                              ("escaped_dep", "UnresolvedConsumedSource:"),
                              ("transient_extern", "TransientExtern:"),
                              ("transient_consumed", "TransientConsumedSource:")):
            with self.subTest(case=case):
                _, record, _ = self.nested_result(case, 2)
                self.assertTrue(any(b.startswith(blocker) for b in record["blockers"]), record["blockers"])
        # Existing ordinary compiler-failure fixture remains strict, even though feature exit 1 is supported.
        self.prepare("compiler_fails")
        self.assertIn("CompilerFailed", self.record_result(self.invoke("record"))["blockers"])

    def test_custom_build_null_executable_alias_host_target_outdir(self):
        session, record, receipts = self.nested_result()
        events = [json.loads(line) for line in (session / "cargo.stdout.raw").read_text().splitlines()]
        builders = [e for e in events if e["reason"] == "compiler-artifact" and e["target"]["kind"] == ["custom-build"]]
        self.assertEqual(len(builders), 2)
        self.assertTrue(all(e["executable"] is None for e in builders))
        libc = [a for a in record["build_script_associations"] if a["package_id"].endswith("#libc@0.2.184")]
        self.assertEqual(len(libc), 2)
        self.assertEqual(len({a["producer_invocation"] for a in libc}), 1)
        producer = next(r for p, r in receipts if p.parent.name == libc[0]["producer_invocation"])
        self.assertEqual(producer["role"], "Host")
        target_out = session / "target/x86_64-apple-darwin/debug/build/TEST_CODE_output_libc/out"
        generated = [c for c in record["consumed_sources"] if c["path"] == str(target_out / "private.rs")]
        self.assertEqual(len(generated), 1)
        self.assertEqual(generated[0]["owner"]["generated_by"], libc[0]["producer_invocation"])
        self.assertEqual(generated[0]["owner"]["out_dir"], str(target_out))
        self.assertEqual(record["blockers"], [])
        self.assertEqual(len(record["selected_library"]), 1)

    def test_custom_build_alias_and_nested_origin_ambiguity_refuse(self):
        for case, blocker in (("nonnull_mismatch", "CustomBuildExecutable"),
                              ("alias_bytes", "UnresolvedCargoArtifact:"),
                              ("multiple_filenames", "CustomBuildFilenames"),
                              ("metadata_alias", "CustomBuildLinkAlias"),
                              ("duplicate_event", "DuplicateBuildScriptOutDir:"),
                              ("duplicate_producer", "UnresolvedBuildScriptProducer"),
                              ("wrong_package", "UnresolvedCargoArtifact:"),
                              ("missing_origin", "UnresolvedNestedOrigin:"),
                              ("wrong_origin", "UnresolvedNestedOrigin:")):
            with self.subTest(case=case):
                _, record, _ = self.nested_result(case, 2)
                self.assertTrue(any(b.startswith(blocker) for b in record["blockers"]), record["blockers"])
        # The closed alias helper does not accept a transient or failed receipt even with equal bytes.
        event = {"target": {"kind": ["custom-build"], "crate_types": ["bin"]}, "executable": None,
                 "filenames": ["/TEST_CODE/alias"]}
        base = {"kind": "Compile", "exit_code": 0, "blockers": [],
                "outputs": [{"kind": "link", "sha256": "TEST_CODE_SHA", "path": "/TEST_CODE/link"}]}
        for change in ({"kind": "TransientProbe"}, {"exit_code": 1}, {"blockers": ["CompilerFailed"]}):
            with self.assertRaises(owner.Refusal):
                owner.custom_build_producer(event, dict(base, **change), {"/TEST_CODE/alias": "TEST_CODE_SHA"})


    def autocfg_result(self, case="normal", status=0):
        self.prepare("autocfg:" + case)
        result = self.invoke("record")
        self.assertEqual(result.returncode, status, result.stderr.decode(errors="replace"))
        record = self.record_result(result)
        session = Path(json.loads(result.stdout)["record_path"]).parent
        receipts = [(p, json.loads(p.read_text())) for p in (session / "invocations").glob("*/receipt.json")]
        return session, record, receipts

    def test_autocfg_actual_stdin_pipe_and_success_branch(self):
        session, record, receipts = self.autocfg_result()
        probes = sorted(((p, r) for p, r in receipts if r["context"]["kind"] == "NumTraitsAutocfgStdinProbe"),
                        key=lambda pair: pair[1]["context"]["index"])
        self.assertEqual(len(probes), 2)
        self.assertEqual([r["stdin"]["template"] for _, r in probes], ["EmptyStd", "TotalCmp"])
        self.assertEqual(record["blockers"], [])
        self.assertEqual(len(record["nested_origins"]), 3)
        self.assertEqual(len(record["selected_library"]), 1)
        for path, receipt in probes:
            evidence = receipt["stdin"]; body = (path.parent / "stdin.raw").read_bytes()
            self.assertEqual(body, owner.AUTOCFG_BODIES[evidence["template"]])
            self.assertEqual(sha(path.parent / "stdin.raw"), evidence["sha256"])
            self.assertTrue(evidence["eof"]); self.assertFalse(evidence["truncated"])
            observed = json.loads((path.parent / "stderr.raw").read_text())
            self.assertTrue(observed["pipe"])
            self.assertEqual(observed["stdin_hex"], body.hex())
            self.assertEqual(observed["fixture_argv"], [bytes.fromhex(a).decode() for a in receipt["argv_hex"]][1:])
            self.assertGreater((path.parent / "stdout.raw").stat().st_size, 65536)
            self.assertEqual(len(receipt["outputs"]), 1)
            output = receipt["outputs"][0]
            self.assertEqual(output["kind"], "llvm-ir")
            self.assertFalse(Path(output["path"]).exists())
            self.assertEqual(sha(path.parent / output["snapshot"]), output["sha256"])
        self.assertTrue(any(e["name"] == "autocfg" and len(e["producers"]) == 1 for e in record["extern_edges"]))

    def test_autocfg_supported_and_unsupported_closed_branches(self):
        for case, templates, codes in (
            ("nostd", ["EmptyStd", "NoStd", "NoStdTotalCmp"], [1, 0, 0]),
            ("neither", ["EmptyStd", "NoStd", "TotalCmp"], [1, 1, 1]),
            ("expr_unsupported", ["EmptyStd", "TotalCmp"], [0, 1]),
        ):
            with self.subTest(case=case):
                _, record, receipts = self.autocfg_result(case)
                probes = sorted((r for _, r in receipts if r["context"]["kind"] == "NumTraitsAutocfgStdinProbe"),
                                key=lambda r: r["context"]["index"])
                self.assertEqual([r["stdin"]["template"] for r in probes], templates)
                self.assertEqual([r["exit_code"] for r in probes], codes)
                self.assertEqual(record["blockers"], [])
                if case == "expr_unsupported": self.assertEqual(probes[-1]["outputs"], [])
                if case == "neither": self.assertTrue(all(len(r["outputs"]) == 1 for r in probes))

    def test_autocfg_refuses_unapproved_bytes_and_context_before_compiler(self):
        cases = ("body", "newline", "nul", "oversize", "bad_index", "bad_hex", "extra", "native", "target_arg",
                 "emit", "loader", "package", "manifest", "target_env", "outdir", "cwd")
        for case in cases:
            with self.subTest(case=case):
                session, record, _ = self.autocfg_result(case, 2)
                self.assertTrue(any(b.startswith("IncompleteInvocation:") for b in record["blockers"]))
                self.assertFalse(any(p.name.startswith("autocfg-") and p.name != "autocfg-version"
                                     for p in (session / "compiler-entry").iterdir()))
                rejected = [p for p in (session / "invocations").iterdir() if not (p / "receipt.json").exists()]
                self.assertEqual(len(rejected), 1)
                self.assertTrue((rejected[0] / "request.json").is_file())
                self.assertFalse((rejected[0] / "invocation.json").exists())
                if case == "oversize":
                    evidence = json.loads((rejected[0] / "stdin.json").read_text())
                    self.assertEqual((rejected[0] / "stdin.raw").read_bytes(), b"X" * 61)
                    self.assertEqual(evidence["length"], 61)
                    self.assertTrue(evidence["truncated"]); self.assertFalse(evidence["eof"])

    def test_autocfg_evidence_and_transient_authority_fail_closed(self):
        for case, blocker in (("stdin_tamper", "ChangedAutocfgStdin:"), ("stdin_missing", "ChangedAutocfgStdin:"),
                              ("stdin_meta", "ChangedAutocfgStdin:"), ("snapshot_tamper", "ChangedTransientEvidence:"),
                              ("snapshot_missing", "ChangedTransientEvidence:"), ("missing_output", "MissingDeclaredOutput:"),
                              ("symlink", "TransientEvidence:"), ("exit2", "CompilerFailed"),
                              ("signal", "CompilerFailed"), ("version_failure", "CompilerFailed"),
                              ("transient_extern", "TransientExtern:"), ("transient_consumed", "TransientConsumedSource:")):
            with self.subTest(case=case):
                _, record, receipts = self.autocfg_result(case, 2)
                self.assertTrue(any(b.startswith(blocker) for b in record["blockers"]), record["blockers"])
                if case == "exit2": self.assertTrue(any(r["exit_code"] == 2 for _, r in receipts))
                if case == "signal": self.assertTrue(any(r["exit_code"] == -9 for _, r in receipts))

    def test_autocfg_branch_and_actual_helper_origin_must_join(self):
        for case in ("mixed_prefix", "duplicate_index", "wrong_branch", "missing_helper", "wrong_helper", "missing_origin"):
            with self.subTest(case=case):
                _, record, _ = self.autocfg_result(case, 2)
                self.assertTrue(any(b.startswith("AutocfgGraph:") for b in record["blockers"]), record["blockers"])

    def test_writeable_reached_deny_and_hyphen_lints_preserve_original_argv(self):
        self.prepare("writeable_lints")
        result = self.invoke("record")
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        record = self.record_result(result)
        self.assertEqual(record["blockers"], [])
        session = Path(json.loads(result.stdout)["record_path"]).parent
        count = 0
        for path in (session / "invocations").glob("*/receipt.json"):
            receipt = json.loads(path.read_text())
            if receipt["kind"] != "Compile": continue
            args = [bytes.fromhex(a).decode() for a in receipt["argv_hex"]][1:]
            observed = json.loads((path.parent / "stderr.raw").read_text())
            self.assertEqual(observed["fixture_argv"], args)
            self.assertEqual(args[:len(WRITEABLE_LINTS)], WRITEABLE_LINTS)
            count += 1
        self.assertEqual(count, 3)
        for bad in ("--deny=", "--deny=unused=value", "--deny=other::lint", "--deny=unused,dead_code",
                    "--warn=clippy::unnecessary-wraps-extra", "--warn=clippy::or-fun-call/", "--forbid=unused"):
            with self.subTest(argument=bad), self.assertRaises(owner.Refusal):
                owner.parse_rustc([bad])

    def test_transient_declared_paths_cannot_reenter_ordinary_or_selected_authority(self):
        cases = (("autocfg", "artifact_alias"), ("autocfg", "artifact_alias_dot"),
                 ("autocfg", "ordinary_alias"), ("autocfg", "unsupported_artifact_alias"),
                 ("autocfg", "unsupported_ordinary_alias"), ("nested", "artifact_alias"),
                 ("nested", "ordinary_alias"))
        for family, case in cases:
            with self.subTest(family=family, case=case):
                result = self.autocfg_result(case, 2) if family == "autocfg" else self.nested_result(case, 2)
                session, record, receipts = result
                prefix = "TransientOrdinaryOutput:" if "ordinary" in case else "TransientCargoArtifact:"
                self.assertTrue(any(b.startswith(prefix) for b in record["blockers"]), record["blockers"])
                self.assertEqual(record["cargo_exit_code"], 0)
                self.assertEqual(record["selected_library"], [])
                self.assertFalse(any(b.startswith(("CargoDidNotFinish", "ChangedOutput:", "IncompleteInvocation:",
                                                  "UnresolvedBuildScriptProducer", "UnresolvedNestedOrigin:", "AutocfgGraph:"))
                                     for b in record["blockers"]), record["blockers"])
                transient = {o["path"] for _, r in receipts if r["kind"] == "TransientProbe" for o in r["declared_outputs"]}
                app = next(r for _, r in receipts if r["kind"] == "Compile" and r["source"] == str(session / "application/src/lib.rs"))
                self.assertEqual(app["exit_code"], 0)
                self.assertEqual(app["blockers"], [])
                events = [json.loads(line) for line in (session / "cargo.stdout.raw").read_text().splitlines()]
                event = next(e for e in events if e["reason"] == "compiler-artifact" and e["target"]["src_path"] == app["source"])
                if "ordinary" in case:
                    reused = next(o["path"] for o in app["outputs"] if o["path"] in transient)
                    self.assertTrue(all(sha(Path(f)) in {o["sha256"] for o in app["outputs"]} for f in event["filenames"]))
                else:
                    reused = str(Path(event["filenames"][0]).resolve())
                    self.assertIn(reused, transient)
                    self.assertIn(sha(Path(reused)), {o["sha256"] for o in app["outputs"]})
                if case.startswith("unsupported") or family == "nested":
                    self.assertTrue(any(r["kind"] == "TransientProbe" and r["exit_code"] == 1
                                        and reused in {o["path"] for o in r["declared_outputs"]}
                                        and reused not in {o["path"] for o in r["outputs"]} for _, r in receipts))
                self.assertFalse(any(e["path"] in transient and e["producers"] for e in record["extern_edges"]))
                self.assertFalse(any(c["path"] in transient for c in record["consumed_sources"]))
                self.assertFalse(any(str(Path(a["out_dir"]) / name) in transient
                                     for a in record["build_script_associations"] for name in a["generated_files"]))
        for family in ("autocfg", "nested"):
            with self.subTest(nontransient_alias=family):
                session, record, _ = self.autocfg_result("nontransient_alias") if family == "autocfg" else self.nested_result("nontransient_alias")
                self.assertEqual(record["blockers"], [])
                self.assertEqual(record["selected_library"][0]["files"], [str(session / "target/TEST_CODE_legitimate_alias")])


    def fix5_result(self, family, case="normal", status=0):
        self.prepare("fix5:" + family + ":" + case)
        result = self.invoke("record")
        self.assertEqual(result.returncode, status, result.stderr.decode(errors="replace"))
        record = self.record_result(result)
        session = Path(json.loads(result.stdout)["record_path"]).parent
        receipts = [(p.parent, json.loads(p.read_text())) for p in (session / "invocations").glob("*/receipt.json")]
        return session, record, receipts

    def test_framework_exact_literal_has_recording_only_producer_join(self):
        session, record, receipts = self.fix5_result("framework")
        self.assertEqual(record["blockers"], [])
        self.assertEqual(len(record["native_link_declarations"]), 1)
        declaration = record["native_link_declarations"][0]
        self.assertEqual(declaration["state"], "RecordingOnly")
        self.assertEqual(declaration["declaration"], "framework=SystemConfiguration")
        path, receipt = next((p, r) for p, r in receipts if p.name == declaration["consumer_invocation"])
        args = [bytes.fromhex(a).decode() for a in receipt["argv_hex"]]
        self.assertEqual(args[-2:], ["-l", "framework=SystemConfiguration"])
        self.assertEqual(json.loads((path / "stderr.raw").read_text())["fixture_argv"], args[1:])
        self.assertTrue(any(a["producer_invocation"] == declaration["producer_invocation"] for a in record["build_script_associations"]))
        self.assertFalse(any("SystemConfiguration" in c["path"] for c in record["consumed_sources"]))

    def test_framework_finite_argv_and_origin_controls(self):
        for case in ("package", "version", "outdir", "other_framework", "modifier", "attached", "duplicate_flag", "reordered", "output", "source", "link_arg", "native_search"):
            with self.subTest(case=case):
                session, record, _ = self.fix5_result("framework", case, 2)
                self.assertFalse((session / "compiler-entry/compile-system_configuration_sys").exists())
                self.assertTrue(any(b.startswith("IncompleteInvocation:") for b in record["blockers"]))
        for case in ("missing_origin", "wrong_origin", "duplicate_origin", "linked_libs", "linked_paths", "cfg"):
            with self.subTest(case=case):
                session, record, _ = self.fix5_result("framework", case, 2)
                self.assertTrue((session / "compiler-entry/compile-system_configuration_sys").exists())
                self.assertEqual(record["cargo_exit_code"], 0)
                self.assertTrue(any(b.startswith("FrameworkGraph:") or b.startswith("DuplicateBuildScriptOutDir:") for b in record["blockers"]))

    def test_rustix_three_pipe_bodies_eight_status_branches_and_overwrite_chain(self):
        for bits in range(8):
            codes = [(bits >> i) & 1 for i in range(3)]
            with self.subTest(codes=codes):
                session, record, receipts = self.fix5_result("rustix", "codes" + "".join(map(str, codes)))
                self.assertEqual(record["blockers"], [])
                probes = sorted(((p, r) for p, r in receipts if r["context"]["kind"] == "RustixMetadataProbe"), key=lambda pr: pr[1]["stdin"]["template_index"])
                self.assertEqual(len(probes), 3)
                previous, post = None, {"exists": False}
                for index, (path, receipt) in enumerate(probes):
                    self.assertEqual(receipt["exit_code"], codes[index])
                    self.assertEqual((path / "stdin.raw").read_bytes(), owner.RUSTIX_BODIES[index])
                    self.assertTrue(receipt["stdin"]["eof"])
                    self.assertEqual(receipt["predecessor_invocation"], previous)
                    self.assertEqual(owner.state_identity(receipt["metadata_pre"]), owner.state_identity(post))
                    self.assertTrue((session / ("compiler-entry/rustix-fds-" + str(index))).exists())
                    self.assertGreater((path / "stdout.raw").stat().st_size, 65536)
                    self.assertGreater((path / "stderr.raw").stat().st_size, 65536)
                    raw = json.loads((path / "stderr.raw").read_text().splitlines()[-1])
                    self.assertEqual(raw["fixture_argv"], [bytes.fromhex(a).decode() for a in receipt["argv_hex"]][1:])
                    for output in receipt["outputs"]:
                        self.assertTrue(output["observation_only"])
                        self.assertEqual(sha(path / output["snapshot"]), output["sha256"])
                    previous, post = path.name, receipt["metadata_post"]
                transient = probes[0][1]["declared_outputs"][0]["path"]
                self.assertFalse(any(e["path"] == transient and e["producers"] for e in record["extern_edges"]))
                self.assertFalse(any(str(Path(a["out_dir"]) / k) == transient for a in record["build_script_associations"] for k in a["generated_files"]))
        _, record, _ = self.fix5_result("rustix", "remove_failed")
        self.assertEqual(record["blockers"], [])

    def test_rustix_invalid_input_refuses_before_compiler(self):
        cases = ("body", "empty", "no_lf", "extra_lf", "nul", "overflow", "extra", "target_arg", "emit", "output", "output_alias", "native", "package", "version", "target_env", "std", "extra_feature", "encoded", "loader", "outdir", "cwd", "source", "reorder", "preexisting")
        for case in cases:
            with self.subTest(case=case):
                session, record, _ = self.fix5_result("rustix", case, 2)
                self.assertEqual(list((session / "compiler-entry").glob("rustix-*")), [])
                self.assertTrue(any(b.startswith("IncompleteInvocation:") for b in record["blockers"]))
                raw = list((session / "invocations").glob("*/stdin.raw"))
                if case in ("body", "empty", "no_lf", "extra_lf", "nul", "overflow", "reorder"):
                    self.assertEqual(len(raw), 1)
                if case == "overflow":
                    self.assertEqual(raw[0].read_bytes(), b"X" * 123)
                    meta = json.loads(raw[0].with_name("stdin.json").read_text())
                    self.assertFalse(meta["eof"]); self.assertTrue(meta["truncated"])
        for case in ("duplicate", "skip", "prestate"):
            session, record, _ = self.fix5_result("rustix", case, 2)
            self.assertTrue((session / "compiler-entry/rustix-0").exists())
            self.assertFalse((session / "compiler-entry/rustix-1").exists())
            statuses = [json.loads(line) for line in next((session / "target").rglob("statuses.jsonl")).read_text().splitlines()]
            self.assertIn(b"RustixPrestate" if case == "prestate" else b"RustixSequence", bytes.fromhex(statuses[-1]["stderr_hex"]))
            self.assertTrue(any(b.startswith("IncompleteInvocation:") for b in record["blockers"]))

    def test_rustix_evidence_and_source_graph_refuse_with_designated_blockers(self):
        for case in ("tamper_stdin", "tamper_pre", "tamper_post", "tamper_missing_snapshot", "tamper_eof", "tamper_predecessor", "tamper_declaration", "builder_features", "cfg", "missing_origin", "wrong_origin", "duplicate_origin"):
            with self.subTest(case=case):
                _, record, receipts = self.fix5_result("rustix", case, 2)
                self.assertEqual(record["cargo_exit_code"], 0)
                self.assertEqual(sum(r["context"]["kind"] == "RustixMetadataProbe" for _, r in receipts), 3)
                self.assertTrue(any(b.startswith(("ChangedRustixEvidence:", "ChangedTransientEvidence:", "RustixGraph:", "DuplicateBuildScriptOutDir:")) for b in record["blockers"]))
        for case in ("missing_output", "symlink", "exit2", "signal"):
            with self.subTest(case=case):
                _, record, receipts = self.fix5_result("rustix", case, 2)
                probe = next(r for _, r in receipts if r["context"]["kind"] == "RustixMetadataProbe")
                if case in ("missing_output", "symlink"):
                    self.assertEqual(probe["exit_code"], 0)
                    self.assertTrue(any(b.startswith(("MissingDeclaredOutput:", "TransientEvidence:")) for b in probe["blockers"]))
                else:
                    self.assertNotIn(probe["exit_code"], (0, 1)); self.assertIn("CompilerFailed", probe["blockers"])
                self.assertTrue(record["blockers"])

    def test_rustix_declared_path_never_becomes_output_artifact_or_selected_authority(self):
        for case, blocker in (("ordinary_alias", "TransientOrdinaryOutput:"), ("artifact_alias", "TransientCargoArtifact:"), ("artifact_dot", "TransientCargoArtifact:"), ("absent_ordinary_alias", "TransientOrdinaryOutput:"), ("absent_artifact_alias", "TransientCargoArtifact:"), ("extern", "TransientExtern:"), ("consumed", "TransientConsumedSource:")):
            with self.subTest(case=case):
                _, record, receipts = self.fix5_result("rustix", case, 2)
                self.assertEqual(record["cargo_exit_code"], 0)
                self.assertTrue(any(b.startswith(blocker) for b in record["blockers"]))
                probes = [r for _, r in receipts if r["context"]["kind"] == "RustixMetadataProbe"]
                transient = probes[0]["declared_outputs"][0]["path"]
                self.assertTrue(all(r["exit_code"] in (0, 1) and not r["blockers"] for r in probes))
                if case.startswith("absent_"):self.assertTrue(all(not r["metadata_post"]["exists"] for r in probes))
                if "alias" in case or case == "artifact_dot":self.assertEqual(record["selected_library"], [])
                self.assertFalse(any(e["path"] == transient and e["producers"] for e in record["extern_edges"]))
                self.assertFalse(any(c["path"] == transient for c in record["consumed_sources"]))
                self.assertFalse(any(str(Path(a["out_dir"]) / n) == transient for a in record["build_script_associations"] for n in a["generated_files"]))
        _, record, _ = self.fix5_result("rustix", "nontransient_alias")
        self.assertEqual(record["blockers"], []); self.assertEqual(len(record["selected_library"]), 1)


    def prepare_indexmap6(self, case="normal"):
        # Reuse unchanged base inventory setup; replace only this case's fake tools.
        inventory = self.prepare()
        vendor = self.root / "vendor-origin"
        write(vendor / "indexmap/Cargo.toml", INDEXMAP6_MANIFEST)
        write(vendor / "indexmap/src/lib.rs", "// TEST_CODE synthetic indexmap source\n")
        write(vendor / "indexmap/.cargo-checksum.json", '{"files":{},"package":"TEST_CODE"}')
        roots = ["dep", "indexmap"]
        inventory["packages"].append({"id": INDEXMAP6_PACKAGE, "tree": "vendor", "manifest": "indexmap/Cargo.toml"})
        for name in ("equivalent", "hashbrown", "serde_core"):
            roots.append(name)
            write(vendor / name / "Cargo.toml", '[package]\nname="' + name + '"\nversion="0.0.0"\n')
            write(vendor / name / "src/lib.rs", "// TEST_CODE prerequisite " + name + "\n")
            write(vendor / name / ".cargo-checksum.json", '{"files":{},"package":"TEST_CODE"}')
            inventory["packages"].append({"id": "TEST_CODE_" + name, "tree": "vendor", "manifest": name + "/Cargo.toml"})
        inventory["vendor"] = snapshot(vendor, roots)
        rustc = write(self.root / "fake-rustc", "#!" + PYTHON + " -I\n" + INDEXMAP6_RUSTC)
        cargo_text = INDEXMAP6_CARGO.replace("__CASE__", repr(case)).replace("__MUTATIONS__", repr(INDEXMAP6_REJECTIONS)).replace("__TEMPLATE__", repr(INDEXMAP6_ARGS)).replace("__PACKAGE__", repr(INDEXMAP6_PACKAGE))
        cargo = write(self.root / "fake-cargo", "#!" + PYTHON + " -I\n" + cargo_text)
        rustc.chmod(0o700); cargo.chmod(0o700)
        inventory["rustc"] = {"path": str(rustc), "sha256": sha(rustc)}
        inventory["cargo"] = {"path": str(cargo), "sha256": sha(cargo)}
        inventory["generators"]["PROTOC"] = dict(inventory["rustc"])
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly", "profile": owner.PROFILE, "inventory": inventory}))
        return inventory

    def indexmap6_rejection(self, case, marker):
        self.prepare_indexmap6(case)
        result = self.invoke("record")
        self.assertEqual(result.returncode, 2, result.stderr.decode(errors="replace"))
        record = self.record_result(result)
        session = Path(json.loads(result.stdout)["record_path"]).parent
        self.assertEqual(record["selected_library"], [])
        self.assertEqual(record["cargo_exit_code"], 2)
        rejected = list((session / "invocations").iterdir())
        self.assertEqual(len(rejected), 1)
        self.assertEqual({p.name for p in rejected[0].iterdir()}, {"request.json"})
        self.assertEqual(record["blockers"], sorted(["CargoDidNotFinishSuccessfully",
            "IncompleteInvocation:" + rejected[0].name, "UnresolvedSelectedLibrary"]))
        self.assertFalse((session / "compiler-entry").exists())
        self.assertFalse(list((session / "target").rglob("*.rlib")))
        diagnostics = [json.loads(line) for line in (session / "cargo.stderr.raw").read_text().splitlines()]
        self.assertEqual(len(diagnostics), 1)
        self.assertEqual(diagnostics[0]["reason"], "Refused")
        self.assertEqual(diagnostics[0]["detail"], marker)
        request = json.loads((rejected[0] / "request.json").read_text())
        attempt = json.loads((session / "fix6-attempt.json").read_text())
        self.assertEqual(request["argv_hex"], attempt["argv_hex"])
        return session, request, record

    def test_record6_exact_lint_pairs_preserve_parser_input(self):
        base = ["--crate-name", "x", "--out-dir", "/tmp/test", "--emit=dep-info", "src/lib.rs"]
        new = [v for v in INDEXMAP6_LINTS if v != "--allow=clippy::style"]
        old = ["--allow=unknown_but_syntactically_valid", "--warn=unsafe_code", "--deny=private_bounds",
               "--allow=clippy::unnecessary-wraps", "--warn=clippy::or-fun-call",
               "--deny=clippy::branches-sharing-code", "--allow=clippy::alloc-instead-of-core"]
        for literal in new + old:
            with self.subTest(literal=literal):
                args = [literal, *base]; original = list(args)
                parsed = owner.parse_rustc(args)
                prefix, name = literal.split("=", 1)
                self.assertEqual(parsed["options"][{"--allow": "-A", "--warn": "-W", "--deny": "-D"}[prefix]], [name])
                self.assertEqual(args, original)
        args = [*new, "--warn=unsafe_code", "--deny=unsafe-code", "-A", "old-short-form", "-Wold_short", *base]
        original = list(args); parsed = owner.parse_rustc(args)
        self.assertEqual(args, original)
        self.assertEqual(parsed["options"]["-D"], ["unsafe-code", "unreachable-pub", "unnameable-types", "private-interfaces", "private-bounds", "unsafe-code"])
        self.assertEqual(parsed["options"]["-W"], ["rust-2018-idioms", "unsafe_code", "old_short"])
        self.assertEqual(parsed["options"]["-A"], ["old-short-form"])
        for case, (tokens, marker) in INDEXMAP6_REJECTIONS.items():
            with self.subTest(case=case):
                args = [*tokens, *base]; original = list(args)
                with self.assertRaises(owner.Refusal) as raised:
                    owner.parse_rustc(args)
                self.assertEqual(str(raised.exception), marker)
                self.assertEqual(args, original)

    def test_record6_indexmap_argv_receipts_and_source_producers(self):
        inventory = self.prepare_indexmap6()
        result = self.invoke("record")
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        record = self.record_result(result)
        self.assertEqual(record["blockers"], [])
        self.assertEqual(record["review_gate"], "IndependentPolicyReviewRequired")
        self.assertEqual(len(record["selected_library"]), 1)
        session = Path(json.loads(result.stdout)["record_path"]).parent
        paths = list((session / "invocations").glob("*/receipt.json"))
        self.assertEqual(len(paths), 5)
        receipts = [(p, json.loads(p.read_text())) for p in paths]
        path, receipt = next((p, r) for p, r in receipts if r["parsed"]["options"]["--crate-name"] == ["indexmap"])
        expected = [inventory["rustc"]["path"], *[v.format(source=session / "vendor/indexmap/src/lib.rs",
            dest=session / "target/x86_64-apple-darwin/debug/deps", host=session / "target/debug/deps") for v in INDEXMAP6_ARGS]]
        expected_hex = [os.fsencode(v).hex() for v in expected]
        for name in ("request.json", "invocation.json", "receipt.json"):
            self.assertEqual(json.loads((path.parent / name).read_text())["argv_hex"], expected_hex)
        self.assertEqual(json.loads((path.parent / "stderr.raw").read_text())["fixture_argv"], expected[1:])
        self.assertEqual(json.loads((session / "compiler-entry/compile-indexmap").read_text()), expected[1:])
        self.assertEqual(receipt["parsed"]["options"]["-D"], ["unsafe-code", "unreachable-pub", "unnameable-types", "private-interfaces", "private-bounds"])
        self.assertEqual(receipt["parsed"]["options"]["-W"], ["rust-2018-idioms"])
        self.assertEqual(receipt["parsed"]["options"]["--cap-lints"], ["allow"])
        self.assertEqual(receipt["package"], {"id": INDEXMAP6_PACKAGE, "tree": "vendor", "manifest": "indexmap/Cargo.toml"})
        self.assertEqual(receipt["source"], str(session / "vendor/indexmap/src/lib.rs"))
        self.assertEqual(receipt["context"], {"kind": "DirectCargoCompile"})
        self.assertEqual(receipt["role"], "Target")
        self.assertEqual(receipt["exit_code"], 0)
        self.assertEqual(sha(session / "vendor/indexmap/src/lib.rs"), inventory["vendor"]["files"]["indexmap/src/lib.rs"])
        self.assertEqual(sha(session / "vendor/indexmap/Cargo.toml"), inventory["vendor"]["files"]["indexmap/Cargo.toml"])
        self.assertEqual((session / "vendor/indexmap/Cargo.toml").read_text(), INDEXMAP6_MANIFEST)
        edges = [e for e in record["extern_edges"] if e["consumer"] == path.parent.name]
        self.assertEqual(len(edges), 3)
        self.assertEqual({e["name"] for e in edges}, {"equivalent", "hashbrown", "serde_core"})
        self.assertTrue(all(len(e["producers"]) == 1 for e in edges))
        self.assertEqual({p.name for p in (session / "compiler-entry").iterdir()},
                         {"compile-equivalent", "compile-hashbrown", "compile-serde_core", "compile-indexmap", "compile-stock_analysis"})

    def test_record6_lints_do_not_authorize_mismatched_sources(self):
        for case, marker in (("package_mismatch", "UnresolvedSourcePackage"), ("source_mismatch", "SourceMismatch")):
            with self.subTest(case=case):
                session, request, record = self.indexmap6_rejection(case, marker)
                argv = [os.fsdecode(bytes.fromhex(v)) for v in request["argv_hex"]]
                self.assertEqual([v for v in argv if v.startswith(("--deny=", "--warn=", "--allow="))], INDEXMAP6_LINTS)
                self.assertEqual(argv[-2:], ["--cap-lints", "allow"])
                self.assertEqual(record["invocations"], [])
                self.assertEqual((session / "vendor/indexmap/src/lib.rs").read_text(), "// TEST_CODE synthetic indexmap source\n")

    def test_record6_unknown_lints_and_flags_refuse_before_compiler(self):
        for case, (tokens, marker) in INDEXMAP6_REJECTIONS.items():
            with self.subTest(case=case):
                _, request, record = self.indexmap6_rejection(case, marker)
                argv = [os.fsdecode(bytes.fromhex(v)) for v in request["argv_hex"]]
                self.assertEqual(argv[1:1 + len(tokens)], tokens)
                self.assertEqual(record["invocations"], [])

    def prepare_proc_macro7(self, case="normal"):
        inventory = self.prepare()
        vendor = self.root / "vendor-origin"
        roots = ["dep"]
        for crate, name, version in (("serde_derive", "serde_derive", "1.0.228"),
                                     ("tokio_macros", "tokio-macros", "2.7.0")):
            roots.append(name)
            write(vendor / name / "Cargo.toml", '[package]\nname="' + name + '"\nversion="' + version
                  + '"\n[lib]\nname="' + crate + '"\npath="src/lib.rs"\nproc-macro=true\n')
            write(vendor / name / "src/lib.rs", "// TEST_CODE synthetic macro " + name + "\n")
            write(vendor / name / ".cargo-checksum.json", '{"files":{},"package":"TEST_CODE"}')
            inventory["packages"].append({"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
                                          "tree": "vendor", "manifest": name + "/Cargo.toml"})
        for name in ("proc_macro2", "quote", "syn"):
            roots.append(name)
            write(vendor / name / "Cargo.toml", '[package]\nname="' + name + '"\nversion="0.0.0"\n')
            write(vendor / name / "src/lib.rs", "// TEST_CODE prerequisite " + name + "\n")
            write(vendor / name / ".cargo-checksum.json", '{"files":{},"package":"TEST_CODE"}')
            inventory["packages"].append({"id": "TEST_CODE_" + name, "tree": "vendor", "manifest": name + "/Cargo.toml"})
        inventory["vendor"] = snapshot(vendor, roots)
        if case == "origin":
            next(p for p in inventory["packages"] if p["manifest"] == "serde_derive/Cargo.toml")["id"] = "TEST_CODE_foreign"
        sysroot = self.root / "sysroot"
        # Each subcase owns only this new fixture tree; earlier ambiguity cases
        # must not contaminate the next positive control. No budget/policy reset API.
        shutil.rmtree(sysroot / "lib/rustlib", ignore_errors=True)
        for relative in PROC_MACRO7_CANDIDATES:
            write(sysroot / relative, "TEST_CODE sysroot candidate " + relative + "\n")
        if case in ("candidate_missing", "candidate_foreign"):
            (sysroot / PROC_MACRO7_CANDIDATES[0]).unlink()
        if case == "candidate_foreign":
            write(sysroot / PROC_MACRO7_CANDIDATES[0].replace("x86_64-apple-darwin", "aarch64-apple-darwin"), "TEST_CODE foreign")
        if case == "candidate_extra":
            write(sysroot / PROC_MACRO7_CANDIDATES[0].replace("b94f7a67a9654a0b", "other"), "TEST_CODE ambiguous")
        inventory["sysroot"] = snapshot(sysroot, ["lib"])
        rustc = write(self.root / "fake-rustc", "#!" + PYTHON + " -I\n" + PROC_MACRO7_RUSTC)
        text = PROC_MACRO7_CARGO.replace("__CASE__", repr(case)).replace("__TEMPLATES__", repr(PROC_MACRO7_ARGS)).replace("__CANDIDATES__", repr(PROC_MACRO7_CANDIDATES))
        cargo = write(self.root / "fake-cargo", "#!" + PYTHON + " -I\n" + text)
        rustc.chmod(0o700); cargo.chmod(0o700)
        inventory["rustc"] = {"path": str(rustc), "sha256": sha(rustc)}
        inventory["cargo"] = {"path": str(cargo), "sha256": sha(cargo)}
        inventory["generators"]["PROTOC"] = dict(inventory["rustc"])
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly", "profile": owner.PROFILE, "inventory": inventory}))
        return inventory

    def proc_macro7_result(self, case="normal"):
        inventory = self.prepare_proc_macro7(case)
        result = self.invoke("record")
        record = self.record_result(result)
        session = Path(json.loads(result.stdout)["record_path"]).parent
        return inventory, result, record, session

    def proc_macro7_rejection(self, case):
        inventory, result, record, session = self.proc_macro7_result(case)
        self.assertEqual(result.returncode, 2, result.stderr.decode(errors="replace"))
        calls = list((session / "invocations").iterdir())
        self.assertEqual(len(calls), 1)
        self.assertEqual({p.name for p in calls[0].iterdir()}, {"request.json"})
        self.assertEqual(record["invocations"], [])
        self.assertEqual(record["blockers"], sorted(["CargoDidNotFinishSuccessfully", "IncompleteInvocation:" + calls[0].name, "UnresolvedSelectedLibrary"]))
        self.assertFalse((session / "compiler-entry").exists())
        self.assertFalse(list((session / "target").rglob("*.dylib")))
        diagnostic = [json.loads(line) for line in (session / "cargo.stderr.raw").read_text().splitlines()]
        self.assertEqual(diagnostic, [{"schema": owner.SCHEMA, "detail": PROC_MACRO7_REJECTIONS[case], "reason": "Refused", "state": "RecordingOnly"}])
        request = json.loads((calls[0] / "request.json").read_text())
        self.assertEqual(request["argv_hex"], json.loads((session / "fix7-attempt-serde_derive.json").read_text())["argv_hex"])
        return inventory, request, record, session

    def test_record7_bare_proc_macro_declaration_preserves_argv(self):
        inventory, result, record, session = self.proc_macro7_result()
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        self.assertEqual(record["blockers"], [])
        self.assertEqual(record["review_gate"], "IndependentPolicyReviewRequired")
        self.assertEqual(len(record["selected_library"]), 1)
        self.assertEqual(len(record["sysroot_extern_declarations"]), 2)
        self.assertEqual(len(record["extern_edges"]), 6)
        self.assertEqual({p.name for p in (session / "compiler-entry").iterdir()},
                         {"compile-" + n for n in ("proc_macro2", "quote", "syn", "serde_derive", "tokio_macros", "stock_analysis")})
        receipts = [(p.parent, json.loads(p.read_text())) for p in (session / "invocations").glob("*/receipt.json")]
        self.assertEqual(len(receipts), 6)
        for crate, package in (("serde_derive", "serde_derive"), ("tokio_macros", "tokio-macros")):
            with self.subTest(crate=crate):
                call, receipt = next((p, r) for p, r in receipts if r["parsed"]["options"]["--crate-name"] == [crate])
                argv = [inventory["rustc"]["path"], *[v.replace("{session}", str(session)).replace("{sysroot}", inventory["sysroot"]["root"]) for v in PROC_MACRO7_ARGS[crate]]]
                encoded = [os.fsencode(v).hex() for v in argv]
                for name in ("request.json", "invocation.json", "receipt.json"):
                    self.assertEqual(json.loads((call / name).read_text())["argv_hex"], encoded)
                self.assertEqual(json.loads((call / "stderr.raw").read_text())["fixture_argv"], argv[1:])
                self.assertEqual(json.loads((session / ("compiler-entry/compile-" + crate)).read_text()), argv[1:])
                declaration, = receipt["sysroot_extern_declarations"]
                self.assertEqual(declaration, {"kind": "BareProcMacroSearchV1", "name": "proc_macro", "host": owner.TARGET,
                    "compiler_sha256": inventory["rustc"]["sha256"], "sysroot_root": inventory["sysroot"]["root"],
                    "argument_index": argv.index("proc_macro"), "artifact_selection": "not_observed",
                    "candidates": [{"relative_path": p, "sha256": inventory["sysroot"]["files"][p]} for p in PROC_MACRO7_CANDIDATES]})
                self.assertIn(dict(declaration, consumer=call.name), record["sysroot_extern_declarations"])
                self.assertEqual(receipt["source"], str(session / "vendor" / package / "src/lib.rs"))
                self.assertEqual(receipt["role"], "Host"); self.assertEqual(receipt["context"], {"kind": "DirectCargoCompile"})
                self.assertEqual(receipt["exit_code"], 0)
                edges = [e for e in record["extern_edges"] if e["consumer"] == call.name]
                self.assertEqual({e["name"] for e in edges}, {"proc_macro2", "quote", "syn"})
                self.assertTrue(all(len(e["producers"]) == 1 for e in edges))
                self.assertTrue(all(e["path"].startswith(str(session / "target")) for e in edges))
        self.assertFalse(any(e["name"] == "proc_macro" for e in record["extern_edges"]))
        self.assertTrue(all("sysroot_extern_declarations" not in r for _, r in receipts if r["role"] == "Target"))

    def test_record7_bare_proc_macro_rejects_unreached_forms_before_compiler(self):
        for case in ("unknown", "core", "std", "namespace", "modifier", "space", "control", "joined", "duplicate", "mixed",
                     "crate_type", "target", "probe", "out_dir", "search", "emit", "nested"):
            with self.subTest(case=case):
                self.proc_macro7_rejection(case)

    def test_record7_bare_proc_macro_preserves_source_and_sysroot_gates(self):
        for case in ("package", "source", "manifest", "version", "manifest_path", "origin", "candidate_missing",
                     "candidate_drift", "candidate_alias", "candidate_extra", "candidate_foreign"):
            with self.subTest(case=case):
                _, request, _, _ = self.proc_macro7_rejection(case)
                self.assertIn(b"proc_macro".hex(), request["argv_hex"])
        _, result, record, _ = self.proc_macro7_result()
        self.assertEqual(result.returncode, 0); self.assertEqual(record["blockers"], [])
        self.assertEqual(len(record["sysroot_extern_declarations"]), 2)

    def test_record7_declaration_seal_requires_raw_and_macro_artifact_binding(self):
        for case in ("missing", "initial", "all_missing", "nonzero", "role", "source", "package", "raw", "environment", "candidate", "selected",
                     "missing_event", "duplicate_event", "wrong_kind", "wrong_crate_type", "wrong_name", "wrong_source", "wrong_package"):
            with self.subTest(case=case):
                _, result, record, session = self.proc_macro7_result("seal_" + case)
                self.assertEqual(result.returncode, 2, result.stderr.decode(errors="replace"))
                call = (session / "fix7-seal-call").read_text()
                marker = "UnresolvedSysrootExternConsumer:" if case in ("nonzero", "missing_event", "duplicate_event", "wrong_kind", "wrong_crate_type", "wrong_name", "wrong_source", "wrong_package") else "ChangedSysrootExternDeclaration:"
                self.assertIn(marker + call, record["blockers"])
                self.assertFalse(any(d["consumer"] == call for d in record.get("sysroot_extern_declarations", [])))
                self.assertEqual(len(record["sysroot_extern_declarations"]), 1)
                self.assertEqual(record["cargo_exit_code"], 0)
                self.assertEqual(len(list((session / "compiler-entry").iterdir())), 6)
                self.assertEqual(len(record["selected_library"]), 1)
        _, result, record, _ = self.proc_macro7_result()
        self.assertEqual(result.returncode, 0); self.assertEqual(record["blockers"], [])
        self.assertEqual(len(record["sysroot_extern_declarations"]), 2)

    def test_record7_path_qualified_externs_keep_original_graph(self):
        for flag in ("--cfg", "--check-cfg", "-C", "--remap-path-prefix"):
            self.assertEqual(owner.raw_bare_externs(["TEST_CODE_compiler", flag, "--extern=proc_macro"]), [])
        _, result, record, session = self.proc_macro7_result("qualified")
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        self.assertEqual(record["blockers"], [])
        self.assertNotIn("sysroot_extern_declarations", record)
        receipts = [json.loads(p.read_text()) for p in (session / "invocations").glob("*/receipt.json")]
        self.assertTrue(all("sysroot_extern_declarations" not in r for r in receipts))
        self.assertEqual(len(record["extern_edges"]), 8)
        self.assertEqual(len([e for e in record["extern_edges"] if e["name"] == "proc_macro"]), 2)
        self.assertTrue(all(len(e["producers"]) == 1 for e in record["extern_edges"]))
        _, result, record, session = self.proc_macro7_result("unresolved")
        self.assertEqual(result.returncode, 2)
        self.assertIn("UnresolvedExternProducer:" + str(session / "target/debug/deps/unproduced.rlib"), record["blockers"])
        self.assertEqual(len(record["sysroot_extern_declarations"]), 2)
        self.assertEqual(len(record["selected_library"]), 1)
        self.assertFalse(any("SysrootExtern" in b for b in record["blockers"]))


    def prepare_proc_macro8(self, rows, case="normal"):
        inventory = self.prepare()
        rows = [list(row) for row in rows]
        row = rows[0]
        if case == "reject_unknown_package":
            row[0] = "registry+https://github.com/rust-lang/crates.io-index#unknown-macro@0.1.0"
            row[2:4] = ["unknown-macro", "0.1.0"]
        elif case == "reject_unknown_version":
            row[0] = row[0].rsplit("@", 1)[0] + "@0.0.0"
            row[3] = "0.0.0"
        elif case == "reject_crate":
            row[1] = "unknown_macro"
        elif case == "reject_manifest":
            row[4:6] = ["relocated-macro/Cargo.toml", "relocated-macro/src/lib.rs"]
        elif case == "reject_source":
            row[5] = (Path(row[4]).parent / "src/other.rs").as_posix()
        elif case == "reject_guessed_src":
            row[5] = (Path(row[4]).parent / "src/lib.rs").as_posix()
        vendor = self.root / "vendor-origin"
        roots = ["dep"]
        for pid, crate, name, version, manifest, source in rows:
            root = Path(manifest).parent
            roots.append(root.as_posix())
            relative = Path(source).relative_to(root).as_posix()
            write(vendor / manifest, '[package]\nname="' + name + '"\nversion="' + version
                  + '"\n[lib]\nname="' + crate + '"\npath="' + relative + '"\nproc-macro=true\n')
            write(vendor / source, "// TEST_CODE synthetic " + pid + " at " + source + "\n")
            write(vendor / root / ".cargo-checksum.json", '{"files":{},"package":"TEST_CODE"}')
            inventory["packages"].append({"id": pid, "tree": "vendor", "manifest": manifest})
        for name in ("proc_macro2", "quote", "syn"):
            roots.append(name)
            write(vendor / name / "Cargo.toml", '[package]\nname="' + name + '"\nversion="0.0.0"\n')
            write(vendor / name / "src/lib.rs", "// TEST_CODE prerequisite " + name + "\n")
            write(vendor / name / ".cargo-checksum.json", '{"files":{},"package":"TEST_CODE"}')
            inventory["packages"].append({"id": "TEST_CODE_" + name, "tree": "vendor", "manifest": name + "/Cargo.toml"})
        inventory["vendor"] = snapshot(vendor, roots)
        sysroot = self.root / "sysroot"
        shutil.rmtree(sysroot / "lib/rustlib", ignore_errors=True)
        for relative in PROC_MACRO7_CANDIDATES:
            write(sysroot / relative, "TEST_CODE sysroot candidate " + relative + "\n")
        inventory["sysroot"] = snapshot(sysroot, ["lib"])
        rustc = write(self.root / "fake-rustc", "#!" + PYTHON + " -I\n" + PROC_MACRO8_RUSTC)
        cargo_text = PROC_MACRO8_CARGO.replace("__CASE__", repr(case)).replace("__ROWS__", repr(rows)).replace(
            "__TEMPLATES__", repr(PROC_MACRO8_OBSERVED_ARGS))
        cargo = write(self.root / "fake-cargo", "#!" + PYTHON + " -I\n" + cargo_text)
        rustc.chmod(0o700); cargo.chmod(0o700)
        inventory["rustc"] = {"path": str(rustc), "sha256": sha(rustc)}
        inventory["cargo"] = {"path": str(cargo), "sha256": sha(cargo)}
        inventory["generators"]["PROTOC"] = dict(inventory["rustc"])
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly",
                                          "profile": owner.PROFILE, "inventory": inventory}))
        return inventory

    def proc_macro8_result(self, rows, case="normal"):
        inventory = self.prepare_proc_macro8(rows, case)
        result = self.invoke("record")
        record = self.record_result(result)
        session = Path(json.loads(result.stdout)["record_path"]).parent
        return inventory, result, record, session

    def assert_proc_macro8_positive(self, rows, inventory, result, record, session):
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        self.assertEqual(record["blockers"], [])
        self.assertEqual(record["review_gate"], "IndependentPolicyReviewRequired")
        self.assertEqual(len(record["selected_library"]), 1)
        self.assertEqual(len(record["sysroot_extern_declarations"]), len(rows))
        self.assertEqual(len(record["extern_edges"]), 3 * len(rows))
        self.assertFalse(any(e["name"] == "proc_macro" for e in record["extern_edges"]))
        receipts = [(p.parent, json.loads(p.read_text())) for p in (session / "invocations").glob("*/receipt.json")]
        self.assertEqual(len(receipts), len(rows) + 4)
        self.assertEqual(len(list((session / "compiler-entry").iterdir())), len(rows) + 4)
        events = [json.loads(line) for line in (session / "cargo.stdout.raw").read_text().splitlines()]
        for pid, crate, name, version, manifest, source in rows:
            with self.subTest(package=pid):
                matches = [(c, r) for c, r in receipts if r["package"]["id"] == pid]
                self.assertEqual(len(matches), 1)
                call, receipt = matches[0]
                self.assertEqual(receipt["package"], {"id": pid, "tree": "vendor", "manifest": manifest})
                self.assertEqual(receipt["source"], str(session / "vendor" / source))
                self.assertEqual(receipt["parsed"]["options"]["--crate-name"], [crate])
                self.assertEqual(receipt["role"], "Host")
                self.assertEqual(receipt["context"], {"kind": "DirectCargoCompile"})
                self.assertEqual(receipt["exit_code"], 0)
                raw = [os.fsdecode(bytes.fromhex(v)) for v in receipt["argv_hex"]]
                suffix = receipt["parsed"]["codegen"]["extra-filename"][0]
                self.assertEqual(json.loads((session / ("compiler-entry/compile-" + crate + suffix)).read_text()), raw[1:])
                self.assertEqual(json.loads((call / "stderr.raw").read_text())["fixture_argv"], raw[1:])
                for filename in ("request.json", "invocation.json"):
                    self.assertEqual(json.loads((call / filename).read_text())["argv_hex"], receipt["argv_hex"])
                declaration, = receipt["sysroot_extern_declarations"]
                self.assertEqual(declaration, {"kind": "BareProcMacroSearchV1", "name": "proc_macro", "host": owner.TARGET,
                    "compiler_sha256": inventory["rustc"]["sha256"], "sysroot_root": inventory["sysroot"]["root"],
                    "argument_index": raw.index("proc_macro"), "artifact_selection": "not_observed",
                    "candidates": [{"relative_path": p, "sha256": inventory["sysroot"]["files"][p]} for p in PROC_MACRO7_CANDIDATES]})
                self.assertIn(dict(declaration, consumer=call.name), record["sysroot_extern_declarations"])
                edges = [e for e in record["extern_edges"] if e["consumer"] == call.name]
                self.assertEqual({e["name"] for e in edges}, {"proc_macro2", "quote", "syn"})
                self.assertTrue(all(len(e["producers"]) == 1 for e in edges))
                event, = [e for e in events if e.get("package_id") == pid]
                self.assertEqual(event["target"], {"src_path": receipt["source"], "kind": ["proc-macro"],
                                                 "crate_types": ["proc-macro"], "name": crate})
                self.assertEqual(set(event["filenames"]), {o["path"] for o in receipt["outputs"] if o["kind"] == "link"})

    def assert_proc_macro8_rejection(self, rows, case, marker):
        inventory, result, record, session = self.proc_macro8_result(rows, case)
        self.assertEqual(result.returncode, 2, result.stderr.decode(errors="replace"))
        calls = list((session / "invocations").iterdir())
        self.assertEqual(len(calls), 1)
        self.assertEqual({p.name for p in calls[0].iterdir()}, {"request.json"})
        self.assertEqual(record["invocations"], [])
        self.assertEqual(record["blockers"], sorted(["CargoDidNotFinishSuccessfully",
                        "IncompleteInvocation:" + calls[0].name, "UnresolvedSelectedLibrary"]))
        self.assertFalse((session / "compiler-entry").exists())
        self.assertFalse(list((session / "target").rglob("*.dylib")))
        diagnostic = [json.loads(line) for line in (session / "cargo.stderr.raw").read_text().splitlines()]
        self.assertEqual(diagnostic, [{"schema": owner.SCHEMA, "detail": marker,
                                      "reason": "Refused", "state": "RecordingOnly"}])
        request = json.loads((calls[0] / "request.json").read_text())
        self.assertEqual(request["argv_hex"], json.loads((session / "fix8-attempt-0.json").read_text())["argv_hex"])
        self.assertIn(b"proc_macro".hex(), request["argv_hex"])
        return inventory, result, record, session

    def test_record8_observed_four_origins_preserve_raw_argv(self):
        rows = tuple(r for r in PROC_MACRO8_ROWS if r[1] in PROC_MACRO8_OBSERVED_ARGS)
        self.assertEqual(len(rows), 4)
        inventory, result, record, session = self.proc_macro8_result(rows, "observed")
        self.assert_proc_macro8_positive(rows, inventory, result, record, session)
        indices = {"futures_macro": 44, "tracing_attributes": 44, "displaydoc": 37, "zerovec_derive": 81}
        for path in (session / "invocations").glob("*/receipt.json"):
            receipt = json.loads(path.read_text());crate = receipt["parsed"]["options"]["--crate-name"][0]
            if crate not in PROC_MACRO8_OBSERVED_ARGS:
                continue
            expected = [inventory["rustc"]["path"], *[v.replace("{session}", str(session)) for v in PROC_MACRO8_OBSERVED_ARGS[crate]]]
            self.assertEqual(receipt["argv_hex"], [os.fsencode(v).hex() for v in expected])
            self.assertEqual(receipt["sysroot_extern_declarations"][0]["argument_index"], indices[crate])

    def test_record8_frozen_catalogue_origins_have_connected_bindings(self):
        self.assertEqual(len(PROC_MACRO8_ROWS), 37)
        self.assertEqual(len({r[0] for r in PROC_MACRO8_ROWS}), 37)
        self.assertEqual(len({r[1] for r in PROC_MACRO8_ROWS}), 35)
        self.assertEqual(owner.BARE_PROC_MACRO_ORIGINS, {(pid, crate): (name, version, manifest, source)
                         for pid, crate, name, version, manifest, source in PROC_MACRO8_ROWS})
        values = self.proc_macro8_result(PROC_MACRO8_ROWS)
        self.assert_proc_macro8_positive(PROC_MACRO8_ROWS, *values)

    def test_record8_versioned_and_nonstandard_sources_stay_distinct(self):
        for crate in ("darling_macro", "thiserror_impl"):
            rows = tuple(r for r in PROC_MACRO8_ROWS if r[1] == crate)
            self.assertEqual(len(rows), 2)
            values = self.proc_macro8_result(rows)
            self.assert_proc_macro8_positive(rows, *values)
            _, result, record, session = self.proc_macro8_result(rows, "seal_swap_versions")
            self.assertEqual(result.returncode, 2)
            calls = json.loads((session / "fix8-macro-calls.json").read_text())
            self.assertEqual(len(calls), 2)
            for call in calls:
                self.assertIn("UnresolvedSysrootExternConsumer:" + call, record["blockers"])
            self.assertNotIn("sysroot_extern_declarations", record)
            self.assertEqual(len(list((session / "compiler-entry").iterdir())), 6)
            self.assertEqual(record["cargo_exit_code"], 0)
        rows = tuple(r for r in PROC_MACRO8_ROWS if r[1] == "document_features")
        self.assertEqual(rows[0][5], "document-features/lib.rs")
        values = self.proc_macro8_result(rows)
        self.assert_proc_macro8_positive(rows, *values)
        self.assert_proc_macro8_rejection(rows, "reject_guessed_src", "BareProcMacroOrigin")

    def test_record8_origin_catalogue_rejects_mismatches_before_compiler(self):
        rows = tuple(r for r in PROC_MACRO8_ROWS if r[1] == "futures_macro")
        markers = {"unknown_package": "BareProcMacroOrigin", "unknown_version": "BareProcMacroOrigin",
                   "crate": "BareProcMacroOrigin", "manifest": "BareProcMacroOrigin", "source": "BareProcMacroOrigin",
                   "env_name": "BareProcMacroOrigin", "env_version": "BareProcMacroOrigin", "env_manifest": "BareProcMacroOrigin",
                   "manifest_drift": "BareProcMacroOrigin", "source_drift": "SourceMismatch",
                   "source_package": "UnresolvedSourcePackage"}
        for case, marker in markers.items():
            with self.subTest(case=case, marker=marker):
                self.assert_proc_macro8_rejection(rows, "reject_" + case, marker)
        # Matched control proves the new origin is otherwise accepted.
        self.assert_proc_macro8_positive(rows, *self.proc_macro8_result(rows))

    def test_record8_catalogue_seal_keeps_actual_event_and_extern_gates(self):
        rows = tuple(r for r in PROC_MACRO8_ROWS if r[1] == "futures_macro")
        self.assert_proc_macro8_positive(rows, *self.proc_macro8_result(rows))
        for case in ("missing_event", "duplicate_event", "wrong_package", "wrong_source", "raw", "declaration", "missing_extern"):
            with self.subTest(case=case):
                _, result, record, session = self.proc_macro8_result(rows, "seal_" + case)
                self.assertEqual(result.returncode, 2)
                call, = json.loads((session / "fix8-macro-calls.json").read_text())
                if case == "missing_extern":
                    marker = "UnresolvedExternProducer:" + str(session / "target/debug/deps/libproc_macro2-2e38878fe48e194f.rlib")
                    self.assertEqual(len(record["sysroot_extern_declarations"]), 1)
                    self.assertFalse(any("SysrootExtern" in b for b in record["blockers"]))
                else:
                    marker = ("ChangedSysrootExternDeclaration:" if case in ("raw", "declaration")
                              else "UnresolvedSysrootExternConsumer:") + call
                    self.assertNotIn("sysroot_extern_declarations", record)
                self.assertIn(marker, record["blockers"])
                self.assertEqual(record["cargo_exit_code"], 0)
                self.assertEqual(len(record["selected_library"]), 1)
                self.assertEqual(len(list((session / "compiler-entry").iterdir())), 5)

    def prepare_ring9(self, case="normal"):
        inventory = self.prepare()
        vendor = self.root / "vendor-origin"
        specs = (("ring", "0.17.14"), ("cc", "1.2.59"), ("cfg_if", "0.0.0"),
                 ("getrandom", "0.0.0"), ("untrusted", "0.0.0"))
        for name, version in specs:
            write(vendor / name / "Cargo.toml", '[package]\nname="' + name + '"\nversion="' + version + '"\n')
            write(vendor / name / "src/lib.rs", "// TEST_CODE synthetic " + name + "\n")
            write(vendor / name / ".cargo-checksum.json", '{"files":{},"package":"TEST_CODE"}')
            if name == "ring":
                write(vendor / name / "build.rs", "// TEST_CODE synthetic builder, no native execution\n")
                write(vendor / name / "src/prefixed.rs", "// TEST_CODE prefix correspondence\n")
            inventory["packages"].append({"id": ("registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version)
                                           if name in ("ring", "cc") else "TEST_CODE_" + name,
                                           "tree": "vendor", "manifest": name + "/Cargo.toml"})
        inventory["vendor"] = snapshot(vendor, ["dep", *[n for n, _ in specs]])
        rustc = write(self.root / "fake-rustc", "#!" + PYTHON + " -I\n" + RING9_RUSTC)
        cargo = write(self.root / "fake-cargo", "#!" + PYTHON + " -I\n" + RING9_CARGO.replace("__CASE__", repr(case)).replace("__TEMPLATE__", repr(RING9_ARGS)))
        rustc.chmod(0o700); cargo.chmod(0o700)
        inventory["rustc"] = {"path": str(rustc), "sha256": sha(rustc)}
        inventory["cargo"] = {"path": str(cargo), "sha256": sha(cargo)}
        inventory["generators"]["PROTOC"] = dict(inventory["rustc"])
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly",
                                           "profile": owner.PROFILE, "inventory": inventory}))
        return inventory

    def ring9_result(self, case="normal", status=0):
        self.prepare_ring9(case)
        result = self.invoke("record")
        self.assertEqual(result.returncode, status, result.stderr.decode(errors="replace"))
        record = self.record_result(result)
        session = Path(json.loads(result.stdout)["record_path"]).parent
        receipts = [(p.parent, json.loads(p.read_text())) for p in (session / "invocations").glob("*/receipt.json")]
        return session, record, receipts

    def ring9_before_entry_refusal(self, case, marker):
        session, record, receipts = self.ring9_result(case, 2)
        self.assertFalse((session / "compiler-entry/compile-ring").exists())
        self.assertFalse(any(r.get("context", {}).get("ring_static_declarations") for _, r in receipts))
        diagnostics = [json.loads(line) for line in (session / "cargo.stderr.raw").read_text().splitlines()]
        self.assertTrue(any(d.get("reason") == "Refused" and d.get("detail") == marker for d in diagnostics), diagnostics)
        self.assertEqual(record["native_link_declarations"], [])
        requests = [p for p in (session / "invocations").glob("*/request.json") if not (p.parent / "receipt.json").exists()]
        self.assertEqual(len(requests), 1)
        self.assertEqual(json.loads(requests[0].read_text())["argv_hex"], json.loads((session / "ring9-attempt.json").read_text())["argv_hex"])
        return session, record

    def test_record9_ring_static_template_preserves_raw_argv(self):
        session, record, receipts = self.ring9_result()
        self.assertEqual(record["blockers"], [])
        path, r = next((p, r) for p, r in receipts if r.get("context", {}).get("ring_static_declarations"))
        expected = [str(self.root / "fake-rustc"), *[v.format(source=session / "vendor/ring/src/lib.rs",
            deps=session / "target/x86_64-apple-darwin/debug/deps", host=session / "target/debug/deps",
            out=session / "target/x86_64-apple-darwin/debug/build/ring-a2bdcca0b9c169e7/out") for v in RING9_ARGS]]
        self.assertEqual(len(expected), 54)
        for leaf in ("request.json", "invocation.json", "receipt.json"):
            self.assertEqual(json.loads((path / leaf).read_text())["argv_hex"], [os.fsencode(v).hex() for v in expected])
        self.assertEqual(json.loads((path / "stderr.raw").read_text())["fixture_argv"], expected[1:])
        self.assertEqual(json.loads((session / "compiler-entry/compile-ring").read_text()), expected[1:])
        self.assertEqual(r["parsed"]["options"]["-l"], list(owner.RING_LIBS))
        self.assertEqual(r["parsed"]["options"]["--cfg"], ['feature="' + f + '"' for f in owner.RING_FEATURES])
        self.assertEqual({e["name"] for e in r["externs"]}, {"cfg_if", "getrandom", "untrusted"})
        for case, marker in (("reorder", "RingStaticTemplate"), ("missing", "RingStaticTemplate"),
            ("missing_both_libraries", "RingStaticTemplate"), ("missing_all_native", "RingStaticTemplate"),
            ("attached", "RingStaticTemplate"), ("dynamic", "RingStaticTemplate"),
            ("extra_lib", "RingStaticTemplate"), ("extra_native", "RingCompileContext")):
            with self.subTest(case=case):
                refused_session, _ = self.ring9_before_entry_refusal(case, marker)
                if case in ("missing_both_libraries", "missing_all_native"):
                    attempt = json.loads((refused_session / "ring9-attempt.json").read_text())["argv_hex"]
                    self.assertEqual(len(attempt), 50 if case == "missing_both_libraries" else 48)
                    self.assertNotIn(b"-l".hex(), attempt)
                    self.assertFalse(any(p.name == "compile-ring" for p in (refused_session / "compiler-entry").iterdir()))

    def test_record9_ring_static_connected_declarations_bind_archives(self):
        session, record, receipts = self.ring9_result()
        self.assertEqual(record["blockers"], [])
        self.assertEqual(len(record["native_link_declarations"]), 2)
        by_id = {p.name: (p, r) for p, r in receipts}
        for index, d in enumerate(record["native_link_declarations"]):
            self.assertEqual(d["declaration"], owner.RING_LIBS[index])
            self.assertEqual(d["raw_argument_indices"], [50 + 2 * index, 51 + 2 * index])
            self.assertEqual(d["artifact_selection"], "not_observed")
            self.assertEqual(d["native_child_provenance"], "not_observed")
            self.assertEqual(d["native_producer_qualification"], "not_issued")
            p, r = by_id[d["consumer_invocation"]]
            self.assertEqual(r["exit_code"], 0)
            self.assertEqual(by_id[d["producer_invocation"]][1]["role"], "Host")
            cc = next(e for e in record["extern_edges"] if e["consumer"] == d["producer_invocation"] and e["name"] == "cc")
            self.assertEqual(len(cc["producers"]), 1)
            self.assertEqual(by_id[cc["producers"][0]][1]["package"]["id"], "registry+https://github.com/rust-lang/crates.io-index#cc@1.2.59")
            for phase in ("pre", "post"):
                o = d["archive_" + phase]
                self.assertEqual((p / o["snapshot"]).read_bytes(), b"TEST_CODE_ARCHIVE_" + str(index).encode())
                self.assertEqual(sha(p / o["snapshot"]), o["sha256"])
                self.assertEqual(Path(o["path"]).stat().st_size, o["length"])
                self.assertFalse(any(v["path"] == o["path"] for _, rr in receipts for v in rr["outputs"]))
        session, record, receipts = self.ring9_result("compiler_fail", 2)
        p, r = next((p, r) for p, r in receipts if r.get("context", {}).get("ring_static_declarations"))
        self.assertEqual(r["exit_code"], 7)
        self.assertEqual(r["blockers"], ["CompilerFailed"])
        self.assertEqual(len(r["ring_archive_pre"]), 2);self.assertEqual(len(r["ring_archive_post"]), 2)
        self.assertEqual(record["native_link_declarations"], [])
        self.assertIn("RingGraph:RingConsumer", record["blockers"])
        self.assertTrue((p / "stderr.raw").read_bytes())

    def test_record9_ring_static_origin_environment_and_sources_refuse(self):
        cases = (("package", "FixedPackageContext"), ("version", "FixedPackageContext"),
            ("cwd", "FixedPackageContext"), ("manifest", "RingEnvironmentContext"),
            ("source_hash", "FixedPackageSource"), ("source_arg", "RingCompileContext"),
            ("feature_missing", "RingFeatureContext"), ("feature_extra", "RingFeatureContext"),
            ("host", "RingCompileContext"), ("wrapper", "RingEnvironmentContext"),
            ("loader", "CompilerEnvironmentInjection"), ("environment", "RingEnvironmentContext"),
            ("outdir", "RingStaticTemplate"), ("link_arg", "RingCompileContext"),
            ("unknown_flag", "UnsupportedRustcArgument:-Zunknown"))
        for case, marker in cases:
            with self.subTest(case=case):self.ring9_before_entry_refusal(case, marker)

    def test_record9_ring_static_archive_snapshots_are_required(self):
        for case in ("pre_missing", "pre_symlink", "pre_hardlink"):
            with self.subTest(case=case):self.ring9_before_entry_refusal(case, "RingArchiveEvidence")
        for case, marker in (("during_change", "RingArchiveChanged"), ("post_missing", "RingArchiveEvidence")):
            with self.subTest(case=case):
                session, record, receipts = self.ring9_result(case, 2)
                p, r = next((p, r) for p, r in receipts if r.get("context", {}).get("ring_static_declarations"))
                self.assertEqual(r["exit_code"], 0);self.assertIn(marker, r["blockers"])
                self.assertTrue((session / "compiler-entry/compile-ring").exists())
                self.assertTrue((p / r["ring_archive_pre"][0]["snapshot"]).is_file())
                if case == "during_change":self.assertNotEqual(r["ring_archive_pre"][0]["sha256"], r["ring_archive_post"][0]["sha256"])
                self.assertEqual(record["native_link_declarations"], [])
        for case in ("snapshot_missing", "snapshot_changed", "snapshot_swapped", "final_archive"):
            with self.subTest(case=case):
                session, record, _ = self.ring9_result(case, 2)
                self.assertEqual(record["cargo_exit_code"], 0)
                self.assertIn("RingGraph:RingArchiveBinding", record["blockers"])
                self.assertEqual(record["native_link_declarations"], [])

    def test_record9_ring_static_final_events_and_edges_are_unique(self):
        for case, marker, length in (("seal_missing_both_libraries", "RingStaticTemplate", 50),
                ("seal_missing_all_native", "RingStaticTemplate", 48),
                ("seal_unannotated_full", "RingInvocationEvidence", 54)):
            with self.subTest(case=case):
                session, record, receipts = self.ring9_result(case, 2)
                call, receipt = next((p, r) for p, r in receipts if r.get("source") == str(session / "vendor/ring/src/lib.rs"))
                self.assertEqual(record["cargo_exit_code"], 0)
                self.assertEqual(receipt["exit_code"], 0)
                self.assertEqual(len(json.loads((session / "compiler-entry/compile-ring").read_text())), 53)
                self.assertIn("RingGraph:" + marker, record["blockers"])
                self.assertFalse(any(b.startswith("IncompleteInvocation:") for b in record["blockers"]))
                self.assertEqual(record["native_link_declarations"], [])
                for leaf in ("request.json", "invocation.json", "receipt.json"):
                    data = json.loads((call / leaf).read_text())
                    self.assertEqual(len(data["argv_hex"]), length)
                    self.assertNotIn("ring_static_declarations", data.get("context", {}))
                    self.assertFalse(any(k.startswith("ring_archive_") for k in data))
                    self.assertEqual(data["environment_hex"][b"CARGO_PKG_NAME".hex()], b"ring".hex())
        cases = {"missing_origin": "RingOrigin", "duplicate_origin": "RingOrigin", "origin_package": "RingOrigin",
            "origin_outdir": "RingOrigin", "missing_builder": "RingOrigin", "duplicate_builder": "RingOrigin",
            "builder_package": "RingOrigin", "builder_features": "RingBuilderFeatures", "cc_edge": "RingBuilderCc",
            "linked_libs": "RingDeclaration", "linked_paths": "RingDeclaration", "event_cfg": "RingDeclaration",
            "event_env": "RingDeclaration", "missing_consumer": "RingConsumer", "duplicate_consumer": "RingConsumer",
            "consumer_features": "RingConsumer", "consumer_role": "RingConsumer", "consumer_source": "RingConsumer",
            "extern_edge": "RingConsumer", "context_tamper": "RingInvocationEvidence"}
        for case, marker in cases.items():
            with self.subTest(case=case):
                session, record, _ = self.ring9_result(case, 2)
                self.assertTrue((session / "compiler-entry/compile-ring").exists())
                self.assertEqual(record["cargo_exit_code"], 0)
                self.assertIn("RingGraph:" + marker, record["blockers"])
                self.assertEqual(record["native_link_declarations"], [])
                if case in ("cc_edge", "extern_edge"):
                    self.assertTrue(any(b.startswith("UnresolvedExternProducer:") for b in record["blockers"]))

    def test_record9_ring_static_does_not_generalize_native_authority(self):
        session, record, receipts = self.ring9_result("host_probe_control")
        self.assertEqual(record["blockers"], [])
        self.assertEqual(len(record["native_link_declarations"]), 2)
        builder = next(r for _, r in receipts if r.get("source") == str(session / "vendor/ring/build.rs"))
        probe = next(r for _, r in receipts if r["kind"] == "Probe")
        for r in (builder, probe):
            self.assertEqual(r["exit_code"], 0)
            self.assertNotIn("ring_static_declarations", r["context"])
            self.assertEqual(r["environment_hex"][b"CARGO_PKG_NAME".hex()], b"ring".hex())
        self.assertEqual(builder["role"], "Host")
        self.assertTrue((session / "compiler-entry/compile-build_script_build").exists())
        self.assertEqual(json.loads((session / "compiler-entry/probe-ring").read_text()), ["--print", "sysroot"])
        self.ring9_before_entry_refusal("package", "FixedPackageContext")
        self.ring9_before_entry_refusal("foreign_lib", "RingStaticTemplate")
        for case in ("archive_output", "selected_archive"):
            with self.subTest(case=case):
                session, record, _ = self.ring9_result(case, 2)
                self.assertEqual(record["native_link_declarations"], [])
                self.assertTrue(any(b in ("RingGraph:RingArchiveBinding", "UnresolvedSelectedLibrary") for b in record["blockers"]))
                self.assertNotIn("qualified", record)
        _, record, _ = self.fix5_result("framework")
        self.assertEqual(record["blockers"], [])
        self.assertEqual([d["declaration"] for d in record["native_link_declarations"]], [owner.FRAMEWORK_LITERAL])
        for args in (["-l", "static=other"], ["-lother"], ["--extern-native", "other"]):
            with self.subTest(args=args), self.assertRaisesRegex(owner.Refusal, "UnsupportedRustcArgument"):
                owner.parse_rustc(args)

    def prepare_d1(self, case="normal"):
        inventory = self.prepare()
        vendor = self.root / "vendor-origin"
        specs = (("libsqlite3-sys", "0.28.0"), ("cc", "1.2.59"), ("diesel", "2.3.7"), ("rusqlite", "0.31.0"))
        for name, version in specs:
            write(vendor / name / "Cargo.toml", '[package]\nname="' + name + '"\nversion="' + version + '"\n')
            write(vendor / name / "src/lib.rs", "// TEST_CODE " + name + "\n")
            write(vendor / name / ".cargo-checksum.json", '{"files":{},"package":"TEST_CODE"}')
            inventory["packages"].append({"id":"registry+https://github.com/rust-lang/crates.io-index#"+name+"@"+version,
                                           "tree":"vendor","manifest":name+"/Cargo.toml"})
        for leaf in ("build.rs", "sqlite3/sqlite3.c", "sqlite3/sqlite3.h", "sqlite3/bindgen_bundled_version.rs"):
            write(vendor / "libsqlite3-sys" / leaf, "// TEST_CODE fixed member " + leaf + "\n")
        for leaf in ("src/tool.rs", "src/tempfile.rs"):
            write(vendor / "cc" / leaf, "// TEST_CODE cc " + leaf + "\n")
        (vendor / "cc/src/detect_compiler_family.c").write_bytes(D1_PROBE)
        self.assertEqual(len(D1_PROBE), 206)
        self.assertEqual(hashlib.sha256(D1_PROBE).hexdigest(), owner.PROBE_DIGEST)
        inventory["vendor"] = snapshot(vendor, ["dep", *[name for name, _ in specs]])
        sysroot = self.root / "sysroot"
        write(sysroot / "lib/rustlib/x86_64-apple-darwin/lib/test.bin", "TEST_CODE host lib")
        inventory["sysroot"] = snapshot(sysroot, ["lib"])
        for name, body in (("fake-native-cc", D1_NATIVE), ("fake-native-ar", D1_NATIVE),
                           ("fake-rustc", D1_RUSTC.replace("__BUILDER__", repr(D1_BUILDER))),
                           ("fake-cargo", D1_CARGO.replace("__CASE__", repr(case)).replace("__FEATURES__", repr(D1_FEATURES)))):
            path = write(self.root / name, "#!" + PYTHON + " -I\n" + body);path.chmod(0o700)
        for role, name in (("CC", "fake-native-cc"), ("AR", "fake-native-ar")):
            path = self.root / name
            inventory["generators"][role] = {"path":str(path),"sha256":sha(path)}
            inventory["environment"][role] = str(path)
        for key, name in (("rustc", "fake-rustc"), ("cargo", "fake-cargo")):
            inventory[key] = {"path":str(self.root / name),"sha256":sha(self.root / name)}
        inventory["generators"]["PROTOC"] = dict(inventory["rustc"])
        inventory["environment"]["PROTOC"] = inventory["rustc"]["path"]
        if case == "capture_fault":
            # Isolated copied-owner fault site only; real child still runs and later calls succeed.
            body = self.tool.read_text()
            needle = 'try:dest=open(call/(name+".raw"),"xb")'
            self.assertEqual(body.count(needle), 1)
            body = body.replace(needle, 'try:\n                if name=="stderr" and "-E" in argv:raise OSError("TEST_CODE sink failure")\n                dest=open(call/(name+".raw"),"xb")')
            self.tool.write_text(body)
            inventory["owner_sha256"] = sha(self.tool)
        self.policy.write_text(json.dumps({"schema":owner.SCHEMA,"mode":"RecordingOnly",
                                           "profile":owner.BUNDLED_PROFILE,"inventory":inventory}))
        return inventory

    def d1_result(self, case="normal", code=0):
        inventory = self.prepare_d1(case)
        run = self.invoke("record")
        self.assertEqual(run.returncode, code, run.stdout.decode(errors="replace") + run.stderr.decode(errors="replace"))
        record = self.record_result(run)
        session = Path(json.loads(run.stdout)["record_path"]).parent
        native = json.loads((session / "native-record.json").read_text())
        calls = [(p.parent, json.loads(p.read_text())) for p in (session / "native-invocations").glob("*/receipt.json")]
        self.assertEqual(record["native_record_sha256"], sha(session / "native-record.json"))
        return inventory, session, record, native, calls

    def d1_refusal(self, case, marker, entries=None):
        values = self.d1_result(case, 2)
        _, session, record, native, calls = values
        reasons = [f for _, r in calls for f in r["failures"]] + native["blockers"]
        self.assertTrue(any(marker in reason for reason in reasons), reasons)
        self.assertTrue(native["blockers"])
        self.assertNotIn("final_archive_writer", native)
        hits = list((session / "native-entry").iterdir()) if (session / "native-entry").exists() else []
        if entries is not None:self.assertEqual(len(hits), entries)
        attempts = [json.loads(line) for line in (session / "native-attempts.jsonl").read_text().splitlines()]
        for attempt in attempts:
            self.assertTrue(any(r["args_hex"] == attempt["args_hex"] for _, r in calls))
        for _, r in calls:
            self.assertNotIn("qualified", r)
        return values

    def test_d1_darwin_private_pipe_identity_and_receipts(self):
        import ctypes
        import fcntl
        from unittest import mock
        self.assertEqual(sys.platform, "darwin", "fixed D1 pipe ABI requires Darwin; no fallback")
        opened = []
        try:
            read, write = os.pipe(); opened.extend((read, write))
            other_read, other_write = os.pipe(); opened.extend((other_read, other_write))
            dup_read, dup_write = os.dup(read), os.dup(write); opened.extend((dup_read, dup_write))
            os.set_blocking(read, False)
            os.write(write, b"J")
            flags = {fd:fcntl.fcntl(fd, fcntl.F_GETFL) for fd in opened}
            inheritance = {fd:os.get_inheritable(fd) for fd in opened}
            original = owner.native_jobserver_identity((read, write))
            duplicate = owner.native_jobserver_identity((dup_read, dup_write))
            for observed in (original, duplicate):
                self.assertEqual(observed["pid"], os.getpid())
                self.assertEqual((observed["flavor"], observed["buffer_bytes"]), (6, 184))
                left, right = observed["endpoints"]
                self.assertEqual(left["handle"], right["peer"])
                self.assertEqual(right["handle"], left["peer"])
                for end in (left, right):
                    self.assertEqual(end["returned_bytes"], 184)
                    self.assertEqual(end["handle"], os.fstat(end["fd"]).st_ino)
            self.assertEqual([e["handle"] for e in original["endpoints"]],
                             [e["handle"] for e in duplicate["endpoints"]])
            for pair, marker in (((write, read), "InvalidJobserverDescriptors"),
                                 ((read, other_write), "NativeJobserverPair")):
                with self.subTest(pair=pair), self.assertRaisesRegex(owner.Refusal, "^"+marker+"$"):
                    owner.native_jobserver_identity(pair)
            os.close(dup_read); opened.remove(dup_read)
            with self.assertRaisesRegex(owner.Refusal, "^InvalidJobserverDescriptors$"):
                owner.native_jobserver_identity((dup_read, write))
            for count in (0, 183, 185):
                with self.subTest(returned_bytes=count), mock.patch.object(ctypes, "CDLL") as library:
                    query = library.return_value.proc_pidfdinfo
                    query.return_value = count
                    with self.assertRaisesRegex(owner.Refusal, "^NativeJobserverIdentityQuery$"):
                        owner.native_jobserver_identity((read, write))
                    self.assertEqual(query.call_count, 1)
                    args = query.call_args.args
                    self.assertEqual((args[0], args[1], args[2], args[4]), (os.getpid(), read, 6, 184))
                    library.assert_called_once_with("/usr/lib/libproc.dylib", use_errno=True)
            self.assertEqual(os.read(read, 2), b"J")
            with self.assertRaises(BlockingIOError):os.read(read, 1)
            self.assertTrue(all(fcntl.fcntl(fd, fcntl.F_GETFL) == flags[fd] for fd in opened))
            self.assertTrue(all(os.get_inheritable(fd) == inheritance[fd] for fd in opened))
        finally:
            for fd in opened:os.close(fd)
        _, _, record, native, calls = self.d1_result()
        self.assertEqual(record["blockers"], [])
        forwarded = [r for _, r in calls if not r["owner_issued_inspector"]]
        inspectors = [r for _, r in calls if r["owner_issued_inspector"]]
        self.assertEqual((len(forwarded), len(inspectors)), (4, 2))
        for receipt in forwarded:
            identity = receipt["jobserver_identity"]
            self.assertEqual(identity["state"], "RecordingOnly")
            self.assertEqual((identity["flavor"], identity["buffer_bytes"]), (6, 184))
            left, right = identity["endpoints"]
            self.assertEqual(left["handle"], right["peer"])
            self.assertEqual(right["handle"], left["peer"])
            self.assertTrue(all(e["returned_bytes"] == 184 and e["handle"] == e["inode"] for e in (left, right)))
        for receipt in inspectors:
            self.assertIsNone(receipt["jobserver_identity"])
            for name in ("CARGO_MAKEFLAGS", "MAKEFLAGS", "MFLAGS"):
                self.assertNotIn(name.encode().hex(), receipt["environment_hex"])

    def test_d1_connected_native_graph_is_recording_only(self):
        inv, session, record, native, calls = self.d1_result()
        self.assertEqual(record["blockers"], []);self.assertEqual(native["blockers"], [])
        self.assertEqual(len(calls), 6)
        self.assertEqual({r["operation"]["class"] for _, r in calls},
            {"CompilerFamilyFileProbe", "SqliteObjectCompile", "ArchiveAppend", "ArchiveIndex", "ArchiveInspector"})
        self.assertEqual(native["artifact_selection"], "not_observed")
        self.assertEqual(native["native_producer_qualification"], "not_issued")
        self.assertEqual(native["static_declaration"], "static=sqlite3")
        # Existing raw receipts identify their directories; final graph owns the ids.
        rust = {p.parent.name:json.loads(p.read_text()) for p in (session / "invocations").glob("*/receipt.json")}
        self.assertEqual(rust[native["builder"]]["role"], "Host")
        self.assertEqual(rust[native["cc_producer"]]["package"]["id"], owner.CC_PACKAGE)
        self.assertEqual(rust[native["rust_consumer"]]["parsed"]["options"]["-l"], ["static=sqlite3"])
        self.assertTrue(any(e["producers"] == [native["rust_consumer"]] for e in record["extern_edges"]))
        for call, r in calls:
            self.assertEqual(r["protocol_state"], "Completed");self.assertEqual(r["tool_result"], 0)
            self.assertEqual(json.loads((call / "request.json").read_text())["args_hex"], r["args_hex"])
            self.assertEqual(sha(call / "stdout.raw"), r["stdout_sha256"])
            self.assertEqual(r["environment_hex"][b"CARGO_ENCODED_RUSTFLAGS".hex()], "")
        hits = [json.loads(p.read_text()) for p in (session / "native-entry").iterdir()]
        self.assertEqual(len(hits), 6)
        self.assertTrue(all(h["encoded_flags"] == "" for h in hits))
        self.assertNotIn("accepted", record);self.assertEqual(record["review_gate"], "IndependentPolicyReviewRequired")

    def test_d1_transient_probe_retirement_retry_and_survival(self):
        for case, state, probes in (("normal","RetiredAfterCcReturn",1), ("probe_survives","Present",1),
                                     ("probe_retry","RetiredAfterCcReturn",2)):
            with self.subTest(case=case):
                _, session, record, native, calls = self.d1_result(case)
                detected = [(c,r) for c,r in calls if r["operation"]["class"] == "CompilerFamilyFileProbe"]
                self.assertEqual(len(detected), probes)
                for c,r in detected:
                    item = next(v for v in native["operations"] if v["operation_id"] == c.name)
                    self.assertEqual(item["input_final_state"], state)
                    self.assertEqual((c / "input-pre.raw").read_bytes(), D1_PROBE)
                    self.assertEqual(r["input_pre"]["identity"], r["input_post"]["identity"])
                if probes == 2:
                    retry = next(r for _,r in detected if r["operation"]["retry"])
                    first = next(r for c,r in detected if c.name == retry["operation"]["predecessor"])
                    self.assertEqual(first["tool_result"], 1)
                self.assertEqual(record["blockers"], [])

    def test_d1_native_admission_precedes_every_tool_entry(self):
        for case, marker in (("probe_literal","NativeProbeLiteral"), ("probe_overflow","NativeInputExtent"),
                ("probe_path","NativeProbePath"), ("package","FixedPackageContext"),
                ("override","NativeEnvironmentInjection"), ("host_override","NativeEnvironmentInjection"),
                ("encoded_flags","NativeEnvironmentInjection:CARGO_ENCODED_RUSTFLAGS"),
                ("target_flags","NativeEnvironmentInjection:CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS"),
                ("loader","CompilerEnvironmentInjection"), ("fd_reversed","InvalidJobserverDescriptors"),
                ("fd_foreign","NativeJobserverPair"), ("fd_closed","InvalidJobserverDescriptors")):
            with self.subTest(case=case):self.d1_refusal(case, marker, 0)
        self.d1_refusal("unauthorized_retry", "NativeProbePredecessor", 1)

    def test_d1_probe_post_return_and_snapshot_cannot_be_forgiven(self):
        for case, marker in (("probe_post_missing","NativeInputMissing"), ("probe_post_change","NativeInputChanged"),
                             ("snapshot_drift","NativeSnapshotChanged")):
            with self.subTest(case=case):self.d1_refusal(case, marker)
        # Permanent-object drift has no transient-retirement exemption.
        self.d1_refusal("object_drift", "NativeVersionChanged")

    def test_d1_archive_versions_include_real_failed_attempt(self):
        for case, modes in (("normal",["cqD","sD"]), ("fallback",["cqD","cq","s"]),
                             ("identical_index",["cqD","sD"])):
            with self.subTest(case=case):
                _, _, record, native, calls = self.d1_result(case)
                by_id = {c.name:r for c,r in calls};chain = native["archive_chain"]
                self.assertEqual([by_id[i]["operation"]["mode"] for i in chain], modes)
                self.assertEqual(native["final_archive_writer"], chain[-1])
                self.assertEqual(len(set(chain)), len(chain))
                for prev, current in zip(chain, chain[1:]):
                    self.assertEqual(by_id[current]["operation"]["predecessor"], prev)
                    self.assertEqual(owner.native_state_key(by_id[current]["archive_pre"]), owner.native_state_key(by_id[prev]["archive_post"]))
                if case == "fallback":self.assertEqual(by_id[chain[0]]["tool_result"], 3)
                if case == "identical_index":self.assertEqual(by_id[chain[0]]["archive_post"]["sha256"], by_id[chain[1]]["archive_post"]["sha256"])
                self.assertEqual(record["blockers"], [])

    def test_d1_archive_invalid_edges_and_partial_fallback_refuse(self):
        for case, marker in (("archive_initial","NativeArchiveInitial"), ("predecessor_change","NativeVersionChanged"),
                ("extra_index","NativeArchiveTransition"), ("index_fail","NativeArchiveIndex"),
                ("partial_fallback","NativeInspectorFailed"), ("compile_fail","NativeObjectProducer"),
                ("archive_drift","NativeVersionChanged")):
            with self.subTest(case=case):self.d1_refusal(case, marker)

    def test_d1_inspector_exact_members_bytes_and_diagnostics(self):
        for case, marker in (("foreign_member","NativeInspectorMembers"), ("pseudo_member","NativeInspectorFailed"),
                ("duplicate_member","NativeInspectorFailed"), ("extract_change","NativeInspectorObject"),
                ("extract_overflow","NativeInspectorFailed"), ("inspector_diagnostic","NativeInspectorFailed"),
                ("inspector_archive_change","NativeInspectorFailed")):
            with self.subTest(case=case):
                _, session, record, native, calls = self.d1_refusal(case, marker)
                self.assertEqual(record["cargo_exit_code"], 0)
                self.assertTrue(any(r["operation"]["class"] == "ArchiveInspector" for _,r in calls))
                self.assertTrue((session / "native-inspector-owner.json").is_file())

    def test_d1_subprocess_fds_eof_concurrent_streams_and_inspector_origin(self):
        _, session, record, native, calls = self.d1_result("streams")
        hits = [json.loads(p.read_text()) for p in (session / "native-entry").iterdir()]
        self.assertEqual(len(hits), 6)
        self.assertTrue(all(h["stdin_eof"] and h["canary_closed"] for h in hits))
        self.assertEqual(sum(h["jobserver"] for h in hits), 4)
        obj = next((c,r) for c,r in calls if r["operation"]["class"] == "SqliteObjectCompile")
        self.assertEqual((obj[0]/"stdout.raw").read_bytes(), b"O"*100000)
        self.assertEqual((obj[0]/"stderr.raw").read_bytes(), b"E"*100000)
        forwarded = [r for _,r in calls if not r["owner_issued_inspector"]]
        inspectors = [r for _,r in calls if r["owner_issued_inspector"]]
        self.assertEqual(len(forwarded), 4);self.assertEqual(len(inspectors), 2)
        for r in forwarded:self.assertIn(b"CARGO_MAKEFLAGS".hex(), r["environment_hex"])
        for r in inspectors:
            self.assertNotIn(b"CARGO_MAKEFLAGS".hex(), r["environment_hex"])
            self.assertNotIn(b"MAKEFLAGS".hex(), r["environment_hex"])
        self.assertEqual(record["blockers"], [])

    def test_d1_capture_failure_is_sticky_despite_later_success(self):
        _, _, record, native, calls = self.d1_refusal("capture_fault", "NativeProtocolSticky")
        self.assertEqual(record["cargo_exit_code"], 0)
        failed = [r for _,r in calls if r["protocol_state"] == "ProtocolRefused"]
        self.assertEqual(len(failed), 1);self.assertEqual(failed[0]["tool_result"], 0)
        self.assertTrue(any("NativeCapture:stderr" in f for f in failed[0]["failures"]))
        self.assertTrue(any(r["operation"]["class"] == "ArchiveIndex" and r["tool_result"] == 0 for _,r in calls))

    def test_d1_final_cargo_bindings_and_rust_static_join_are_required(self):
        for case, marker in (("missing_origin","NativeCargoOrigin"), ("duplicate_origin","NativeCargoOrigin"),
                ("missing_builder","NativeCargoOrigin"), ("origin_outdir","NativeCargoOrigin"),
                ("link_directive","NativeLinkDeclaration"), ("bindings_drift","NativeBindings"),
                ("rust_missing_static","NativeRustConsumer")):
            with self.subTest(case=case):self.d1_refusal(case, marker)
        self.d1_refusal("compile_plugin", "NativeCompileFlags")
        self.d1_refusal("compile_source", "NativeCompileSource")
        for case in ("probe_consumed", "probe_relative_consumed", "object_relative_consumed", "archive_relative_consumed"):
            with self.subTest(case=case):
                _, session, record, native, calls = self.d1_refusal(case, "NativeOutputRole", 4)
                self.assertEqual(record["cargo_exit_code"], 0)
                self.assertEqual(native["blockers"], ["NativeSeal:NativeOutputRole"])
                self.assertFalse(any(r["owner_issued_inspector"] for _, r in calls))
                origin = next(a for a in record["build_script_associations"] if a["package_id"] == owner.SQLITE_PACKAGE)
                self.assertIn("libsqlite3.a", origin["generated_files"])
                self.assertNotIn("42detect_compiler_family.c", origin["generated_files"])
                native_names = {"42detect_compiler_family.c", "sqlite3-test.o", "libsqlite3.a"}
                self.assertFalse(any(Path(c["path"]).name in native_names for c in record["consumed_sources"]))
                sqlite = next(json.loads(p.read_text()) for p in (session / "invocations").glob("*/receipt.json")
                              if json.loads(p.read_text()).get("package", {}).get("id") == owner.SQLITE_PACKAGE
                              and json.loads(p.read_text())["role"] == "Target")
                paths = [v for o in sqlite["outputs"] for v in o.get("dep_info", {}).get("paths", [])]
                path = next(v for v in paths if Path(v).name in native_names)
                self.assertEqual(Path(path).is_absolute(), "relative" not in case)
                self.assertTrue((Path(sqlite["cwd"]) / path).resolve().is_file())


    def test_d1_public_cli_and_normal_profile_do_not_grant_native_dispatch(self):
        self.prepare()
        for args in (("record","--features","replay-sqlite-bundled-v1"), ("record",owner.BUNDLED_PROFILE),
                     ("_native","bad","cc","-E","/tmp/foreign")):
            self.assertEqual(self.invoke(*args).returncode, 2)
        result = self.invoke("record")
        self.assertEqual(result.returncode, 0)
        record = self.record_result(result)
        self.assertNotIn("native_record_sha256", record)
        session = Path(json.loads(result.stdout)["record_path"]).parent
        self.assertFalse((session/"native-invocations").exists())

    def prepare_native_e1(self, case="normal"):
        shutil.copyfile(TOOL, self.tool)
        inventory = self.prepare_d1()
        vendor = self.root / "vendor-origin"
        for name, version in (("ring", "0.17.14"), ("psm", "0.1.30")):
            write(vendor / name / "Cargo.toml", '[package]\nname="' + name + '"\nversion="' + version + '"\n')
            write(vendor / name / "build.rs", "// TEST_CODE fixed foreign builder\n")
            write(vendor / name / "src/lib.rs", "// TEST_CODE fixed foreign library\n")
            write(vendor / name / ".cargo-checksum.json", '{"files":{},"package":"TEST_CODE"}')
            inventory["packages"].append({"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
                                           "tree": "vendor", "manifest": name + "/Cargo.toml"})
        write(vendor / "ring/pregenerated/TEST_CODE.s", "// TEST_CODE packaged assembly directory\n")
        write(vendor / "cc/src/command_helpers.rs", "// TEST_CODE fixed cc command closure\n")
        inventory["vendor"] = snapshot(vendor, ["dep", "libsqlite3-sys", "cc", "diesel", "rusqlite", "ring", "psm"])
        rustc_body = D1_RUSTC.replace("__BUILDER__", repr(D1_BUILDER)).replace(
            "inputs=[str(source)]", "inputs=[str(source)]\nif name=='cc':inputs.append(str(source.parent/'detect_compiler_family.c'))")
        for name, body in (("fake-rustc", rustc_body), ("fake-native-cc", E1_NATIVE),
                           ("fake-cargo", E1_CARGO.replace("__CASE__", repr(case)))):
            path = write(self.root / name, "#!" + PYTHON + " -I\n" + body); path.chmod(0o700)
        for key, name in (("rustc", "fake-rustc"), ("cargo", "fake-cargo")):
            inventory[key] = {"path": str(self.root / name), "sha256": sha(self.root / name)}
        inventory["generators"]["CC"] = {"path": str(self.root / "fake-native-cc"), "sha256": sha(self.root / "fake-native-cc")}
        inventory["generators"]["PROTOC"] = dict(inventory["rustc"])
        inventory["environment"]["PROTOC"] = inventory["rustc"]["path"]
        if case in ("capture_fault", "forward_fault"):
            body = self.tool.read_text()
            if case == "capture_fault":
                needle = 'try:dest=open(call/(name+".raw"),"xb")'
                self.assertEqual(body.count(needle), 1)
                body = body.replace(needle, 'try:\n                if env.get("E1_CASE")=="capture_fault" and name=="stderr":raise OSError("TEST_CODE capture")\n                dest=open(call/(name+".raw"),"xb")')
            else:
                needle = 'written = os.write(1 if stream == "stdout" else 2, view)'
                self.assertEqual(body.count(needle), 1)
                body = body.replace(needle, 'raise OSError("TEST_CODE forward")\n                            ' + needle)
            self.tool.write_text(body)
        inventory["owner_sha256"] = sha(self.tool)
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly",
                                           "profile": owner.BUNDLED_PROFILE, "inventory": inventory}))
        return inventory


    def native_e1_result(self, case="normal"):
        inventory = self.prepare_native_e1(case)
        run = self.invoke("record")
        self.assertEqual(run.returncode, 2, run.stdout.decode(errors="replace") + run.stderr.decode(errors="replace"))
        record = self.record_result(run); session = Path(json.loads(run.stdout)["record_path"]).parent
        foreign = json.loads((session / "foreign-native-record.json").read_text())
        calls = [(p.parent, json.loads(p.read_text())) for p in (session / "foreign-native-invocations").glob("*/receipt.json")]
        self.assertEqual(record["foreign_native_record_sha256"], sha(session / "foreign-native-record.json"))
        return inventory, session, record, foreign, calls


    def test_native_e1_two_packages_capture_and_retirement(self):
        for case, final in (("normal", "RetiredAfterCcReturn"), ("survive", "Present")):
            with self.subTest(case=case):
                _, session, record, foreign, calls = self.native_e1_result(case)
                self.assertEqual(record["cargo_exit_code"], 0)
                self.assertEqual(len(calls), 6)
                self.assertTrue(all(r["protocol_state"] == "Completed" for _, r in calls))
                probes = [(c, r) for c, r in calls if r["operation"]["class"] == "CompilerFamilyFileProbe"]
                self.assertEqual({r["context"]["package_id"] for _, r in probes}, {owner.RING_PACKAGE, owner.PSM_PACKAGE})
                for call, r in probes:
                    self.assertEqual((call / "input-pre.raw").read_bytes(), D1_PROBE)
                    self.assertEqual(owner.native_state_key(r["input_pre"]), owner.native_state_key(r["input_post"]))
                    self.assertEqual(r["jobserver_identity"], r["jobserver_return"])
                    retained = next(o for o in foreign["operations"] if o["operation_id"] == call.name)
                    self.assertEqual(retained["input_final_state"], final)
                hits = [json.loads(p.read_text()) for p in (session / "foreign-entry").iterdir()]
                self.assertTrue(all(h["stdin_eof"] for h in hits))
                for hit in hits:
                    receipt = next(r for _, r in calls if [os.fsdecode(bytes.fromhex(a)) for a in r["args_hex"]] == hit["argv"]
                                   and r["context"]["manifest"] == hit["cwd"])
                    self.assertEqual([e["fd"] for e in receipt["jobserver_identity"]["endpoints"]], hit["fds"])


    def test_native_e1_fixed_context_literal_and_live_fd_rejections(self):
        cases = ("name", "version", "component", "manifest", "manifest_absent", "labels_absent", "features",
                 "locale", "out", "target", "branch", "source", "unknown", "literal", "extent", "hardlink",
                 "symlink", "overflow", "fd_missing", "fd_reversed", "fd_foreign", "fd_closed", "compile", "archive")
        for choice in cases:
            with self.subTest(choice=choice):
                _, session, _, foreign, calls = self.native_e1_result("reject_" + choice)
                self.assertEqual(len(calls), 2)
                self.assertTrue(all(r["protocol_state"] == "ProtocolRefused" and r["tool_result"] is None for _, r in calls))
                self.assertFalse((session / "foreign-entry").exists())
                self.assertTrue(any("ForeignProtocolSticky" in b for b in foreign["blockers"]))


    def test_native_e1_warning_only_exact_same_file_retry(self):
        for case in ("warning_stdout", "warning_stderr"):
            with self.subTest(case=case):
                _, _, _, foreign, calls = self.native_e1_result(case)
                probes = [(c, r) for c, r in calls if r["operation"]["class"] == "CompilerFamilyFileProbe"]
                self.assertEqual(len(probes), 4)
                by_id = {c.name: r for c, r in probes}
                for call, r in probes:
                    self.assertEqual(r["protocol_state"], "Completed")
                    if r["operation"]["retry"]:
                        previous = by_id[r["operation"]["predecessor"]]
                        self.assertEqual(previous["tool_result"], 3)
                        self.assertEqual(previous["operation"]["source"], r["operation"]["source"])
                        self.assertTrue(r["source_semantics"]["effective_stdout"])
                self.assertFalse(any("ForeignOperation" in b for b in foreign["blockers"]))
        for case, marker in (("reject_retry", "ForeignProbePredecessor"), ("reject_retry_reordered", "ForeignStageAArgv")):
            with self.subTest(case=case):
                _, session, _, _, calls = self.native_e1_result(case)
                self.assertFalse((session / "foreign-entry").exists())
                self.assertTrue(all(marker in r["failures"] for _, r in calls))
        _, _, _, foreign, calls = self.native_e1_result("duplicate_e")
        self.assertTrue(any("ForeignProtocolSticky" in b for b in foreign["blockers"]))
        self.assertTrue(any(r["protocol_state"] == "ProtocolRefused" and "ForeignProbePredecessor" in r["failures"] for _, r in calls))


    def test_native_e1_raw_status_streams_forward_and_post_faults(self):
        for case, code in (("e_nonzero", 7), ("signal", -15), ("streams", 0)):
            with self.subTest(case=case):
                _, session, _, _, calls = self.native_e1_result(case)
                forwarded = json.loads((session / "foreign-forwarded.json").read_text())
                probes = [(c, r) for c, r in calls if r["operation"]["class"] == "CompilerFamilyFileProbe"]
                for call, r in probes:
                    self.assertEqual((r["protocol_state"], r["tool_result"]), ("Completed", code))
                    observed = next(f for f in forwarded if f["args"] == [os.fsdecode(bytes.fromhex(a)) for a in r["args_hex"]])
                    self.assertEqual(observed["status"], code if code >= 0 else 128 - code)
                    for s in ("stdout", "stderr"):
                        self.assertEqual((call / (s + ".raw")).read_bytes().hex(), observed[s + "_hex"])
                    if case == "streams":
                        self.assertEqual((call / "stdout.raw").read_bytes(), b"O" * 120000)
                        self.assertEqual((call / "stderr.raw").read_bytes(), b"E" * 120000)
        for case in ("capture_fault", "forward_fault", "post_missing", "post_change", "source_post", "snapshot_changed", "snapshot_missing"):
            with self.subTest(case=case):
                _, _, record, foreign, calls = self.native_e1_result(case)
                self.assertEqual(record["cargo_exit_code"], 0)
                self.assertTrue(any("ForeignOperation" in b for b in foreign["blockers"]))
                if case in ("capture_fault", "forward_fault", "post_missing", "post_change"):
                    self.assertTrue(any(r["protocol_state"] == "ProtocolRefused" and r["tool_result"] == 0 for _, r in calls))


    def test_native_e1_help_version_source_semantics_and_ambiguous_concurrency(self):
        for case in ("normal", "help_zero", "version_zig", "version_nonzero", "concurrent", "forward_fault"):
            with self.subTest(case=case):
                _, _, _, foreign, calls = self.native_e1_result(case)
                helpers = [r for _, r in calls if r["operation"]["class"] != "CompilerFamilyFileProbe"]
                self.assertTrue(all(r["operation"]["probe_predecessor"] == "not_observed" for r in helpers))
                if case == "forward_fault":
                    self.assertTrue(any("ForeignProtocolSticky" in b for b in foreign["blockers"]))
                    continue
                self.assertTrue(all(r["protocol_state"] == "Completed" for r in helpers))
                for r in helpers:
                    if r["operation"]["class"] == "CompilerFamilyHelpProbe":
                        self.assertEqual(r["tool_result"], 0 if case == "help_zero" else 9)
                        self.assertEqual(r["source_semantics"]["accepts_cl_style_flags"], case == "help_zero")
                    else:
                        self.assertEqual(r["source_semantics"]["zig_cc"], case == "version_zig")
                        self.assertEqual(r["source_semantics"]["source_nonzero_default"], case == "version_nonzero")
                self.assertTrue(all(g["family_pairing"] == "ambiguous_context_group" for g in foreign["context_groups"]))
                if case == "concurrent":
                    self.assertEqual(len(calls), 6); self.assertEqual(len(helpers), 4)
                    self.assertEqual(len(foreign["context_groups"]), 1)
                    self.assertEqual(len(foreign["context_groups"][0]["operations"]), 6)
        for case in ("extra_help", "attached_help", "extra_version", "wrong_role"):
            with self.subTest(case=case):
                _, session, _, _, calls = self.native_e1_result("reject_" + case)
                self.assertFalse((session / "foreign-entry").exists())
                self.assertTrue(all(r["protocol_state"] == "ProtocolRefused" for _, r in calls))


    def test_native_e1_seal_request_only_and_namespace_ownership(self):
        _, session, record, foreign, calls = self.native_e1_result()
        self.assertEqual(foreign["stage"], "StageAIncomplete")
        self.assertEqual((foreign["native_producer_qualification"], foreign["artifact_selection"]), ("not_issued", "not_observed"))
        self.assertEqual(set(foreign["unclosed"]), {"compiler-family-successors", "family-to-compile", "object", "archive", "builder-run", "consumer"})
        self.assertFalse(any("NativeOutputRole" in b for b in record["blockers"]), record["blockers"])
        literal = str(session / "vendor" / owner.PROBE_LITERAL)
        self.assertTrue(any(c["path"] == literal and c["owner"] == {"tree": "vendor", "relative_path": owner.PROBE_LITERAL}
                            for c in record["consumed_sources"]))
        # Empty-stream ownership stays conservative; no generic 0B/vendor waiver.
        self.assertIn(hashlib.sha256(b"").hexdigest(), foreign["quarantine"]["sha256"])
        for case in ("request_only", "ownership_source", "ownership_snapshot", "ownership_relative", "ownership_output", "ownership_extern", "ownership_artifact",
                     "borrowed_request", "borrowed_initial", "borrowed_target", "borrowed_context"):
            with self.subTest(case=case):
                _, session, record, foreign, calls = self.native_e1_result(case)
                self.assertEqual(record["cargo_exit_code"], 0)
                if case == "request_only":
                    self.assertEqual(len(foreign["operations"]), 6); self.assertEqual(len(calls), 5)
                    self.assertTrue(any("ForeignOperation" in b for b in foreign["blockers"]))
                    requested = [json.loads(p.read_text()) for p in (session / "foreign-native-invocations").glob("*/request.json")]
                    for r in requested:
                        args = [os.fsdecode(bytes.fromhex(a)) for a in r["args_hex"]]
                        if args[:1] == ["-E"]:self.assertIn(args[-1], foreign["quarantine"]["paths"])
                else:
                    self.assertTrue(any("NativeOutputRole" in b for b in record["blockers"]), record["blockers"])
                    denied = set(foreign["quarantine"]["paths"]) | {str(session / "target/copied-native.bin")}
                    self.assertFalse(any(c["path"] in denied for c in record["consumed_sources"]))
                    self.assertFalse(any(f in denied for a in record["selected_library"] for f in a["files"]))
        stream_cases = ["stream_" + stream + "_" + role + "_current"
                        for stream in ("stdout", "stderr") for role in ("source", "output", "extern", "artifact", "generated")]
        stream_cases += ["stream_stdout_source_mutated", "stream_stderr_output_deleted",
                         "stream_stdout_artifact_deleted", "stream_stderr_generated_mutated",
                         "stream_stdout_source_annotation", "stream_stderr_extern_annotation",
                         "stream_stdout_output_requestonly", "stream_stderr_source_requestonly",
                         "stream_stdout_source_mutatedmissingrequest", "stream_stderr_extern_deletedcorruptrequest",
                         "stream_input_source_mutatedmissingrequest", "stream_input_extern_deletedcorruptrequest",
                         "stream_input_source_deletedinvalidstate",
                         "forward_fault", "stream_empty_source_current"]
        for case in stream_cases:
            with self.subTest(case=case):
                _, session, record, foreign, calls = self.native_e1_result(case)
                self.assertEqual(record["cargo_exit_code"], 0)
                controls = json.loads((session / "stream-copy-controls.json").read_text())
                self.assertEqual(len(controls), 2 if case == "forward_fault" else 1)
                for control in controls:
                    path = control["path"]; retained = control["retained_sha256"]; cut = control["cut"]
                    body = bytes.fromhex(control["body_hex"])
                    self.assertEqual(hashlib.sha256(body).hexdigest(), retained)
                    self.assertEqual(bool(body), control["stream"] != "empty")
                    self.assertEqual(Path(path).read_bytes(), body)
                    self.assertNotIn(path, foreign["quarantine"]["paths"])
                    self.assertIn(retained, foreign["quarantine"]["sha256"])
                    self.assertTrue(control["matching_raw_paths"])
                    self.assertTrue(control["matching_receipt_paths"])
                    current = {sha(p) for p in (session / "foreign-native-invocations").glob("*/*.raw")}
                    if cut in ("mutated", "deleted"):
                        # No surviving current raw file may supply the original digest.
                        self.assertNotIn(retained, current)
                        for name in control["matching_raw_paths"]:
                            if cut == "deleted": self.assertFalse(Path(name).exists())
                            else:
                                self.assertNotEqual(sha(Path(name)), retained)
                                self.assertIn(sha(Path(name)), foreign["quarantine"]["sha256"])
                    elif cut == "annotation":
                        for name in control["matching_receipt_paths"]:
                            self.assertNotIn(control["stream"] + "_sha256", json.loads(Path(name).read_text()))
                        self.assertIn(retained, current)
                    elif cut == "requestonly":
                        self.assertTrue(all(not Path(name).exists() for name in control["matching_receipt_paths"]))
                        self.assertIn(retained, current)
                        self.assertTrue(any("ForeignOperation" in b for b in foreign["blockers"]))
                    if control.get("request_cut"):
                        self.assertIn(cut, ("mutated", "deleted"))
                        self.assertNotIn(retained, current)
                        self.assertEqual(len(control["matching_request_paths"]), len(control["matching_receipt_paths"]))
                        for name in control["matching_receipt_paths"]:
                            receipt = json.loads(Path(name).read_text())
                            value = receipt["input_post"]["sha256"] if control["stream"] == "input" else receipt[control["stream"] + "_sha256"]
                            self.assertEqual(value, retained)
                            self.assertTrue(any(b.startswith("ForeignNamespace:" + Path(name).parent.name + ":")
                                                for b in foreign["blockers"]), foreign["blockers"])
                        for name in control["matching_request_paths"]:
                            if control["request_cut"] == "missing": self.assertFalse(Path(name).exists())
                            else:
                                with self.assertRaises(json.JSONDecodeError): json.loads(Path(name).read_bytes())
                    if control["stream"] == "input":
                        self.assertEqual((len(body), retained), (206, owner.PROBE_DIGEST))
                        for name in control["matching_receipt_paths"]:
                            receipt = json.loads(Path(name).read_text())
                            self.assertEqual((receipt["input_post"]["length"], receipt["input_post"]["sha256"]), (206, retained))
                            if control.get("invalid_snapshot_field"):
                                self.assertEqual(receipt["input_pre"], "TEST_CODE_invalid_state")
                                self.assertIn("ForeignNamespace:" + Path(name).parent.name + ":ForeignSnapshotFields", foreign["blockers"])
                        literal = str(session / "vendor" / owner.PROBE_LITERAL)
                        self.assertEqual(Path(literal).read_bytes(), body)
                        self.assertTrue(any(c["path"] == literal and c["owner"] == {"tree": "vendor", "relative_path": owner.PROBE_LITERAL}
                                            for c in record["consumed_sources"]), record["blockers"])
                    if case == "forward_fault":
                        self.assertEqual(control["original_protocol_state"], "ProtocolRefused")
                        self.assertTrue(any("ForeignProtocolSticky" in b for b in foreign["blockers"]))
                    denied = {path, *control.get("artifact_paths", [])}
                    self.assertFalse(any(c["path"] in denied for c in record["consumed_sources"]))
                    self.assertFalse(any(f in denied for a in record["selected_library"] for f in a["files"]))
                    self.assertFalse(any(e["path"] in denied and e["producers"] for e in record["extern_edges"]))
                    role = control["role"]
                    if role == "output":
                        cc_receipts = [(p, json.loads(p.read_text()))
                                       for p in (session / "invocations").glob("*/receipt.json")]
                        cc_receipts = [(p, r) for p, r in cc_receipts
                                       if r.get("source") == str(session / "vendor/cc/src/lib.rs")]
                        self.assertEqual(len(cc_receipts), 1)
                        cc_path, cc = cc_receipts[0]
                        cc_id = cc_path.parent.name
                        sealed_cc = [r for r in record["invocations"] if r["invocation_id"] == cc_id]
                        self.assertEqual(len(sealed_cc), 1)
                        self.assertEqual(sealed_cc[0]["receipt_sha256"], sha(cc_path))
                        self.assertIn("NativeOutputRole:" + cc_id, record["blockers"])
                        self.assertTrue(any(o["path"] == path for o in cc["declared_outputs"]))
                    elif role in ("source", "extern"):
                        self.assertIn("NativeOutputRole:" + path, record["blockers"])
                        if role == "extern":
                            edge = [e for e in record["extern_edges"] if e["path"] == path]
                            self.assertEqual(len(edge), 1); self.assertEqual(edge[0]["producers"], [])
                    elif role == "artifact":
                        self.assertEqual(len(control["artifact_paths"]), 2)
                        self.assertTrue(all(sha(Path(f)) == retained for f in control["artifact_paths"]))
                        self.assertIn("NativeOutputRole:CargoArtifact", record["blockers"])
                        self.assertEqual(record["selected_library"], [])
                    else:
                        self.assertEqual(role, "generated")
                        self.assertIn("NativeOutputRole:GeneratedFile:" + path, record["blockers"])
                        associations = [a for a in record["build_script_associations"] if a["package_id"] == "TEST_CODE_app"]
                        self.assertEqual(len(associations), 1)
                        self.assertEqual(str(Path(path).parent), associations[0]["out_dir"])
                        self.assertNotIn("copied-stream.bin", associations[0]["generated_files"])
                        self.assertEqual(associations[0]["generated_files"]["ordinary.txt"], sha(Path(path).with_name("ordinary.txt")))
                        self.assertNotIn("UnresolvedBuildScriptProducer", record["blockers"])


    def prepare_native_e2(self, case="normal"):
        inventory = self.prepare_native_e1()
        vendor = self.root / "vendor-origin"
        for name, rows in E2_SOURCE_DATA.items():
            for member, _ in rows:write(vendor / name / member, "// TEST_CODE compile input " + member + "\n")
        write(vendor / "ring/include/TEST_CODE.h", "// TEST_CODE pinned include\n")
        for member in E2_PRIVATE_HEADER_MEMBERS:write(vendor / member, "// TEST_CODE private header " + member + "\n")
        for name in ("apple", "llvm", "parser", "generated"):
            write(vendor / ("cc/src/target/" + name + ".rs"), "// TEST_CODE fixed target helper " + name + "\n")
        inventory["vendor"] = snapshot(vendor, ["dep", "libsqlite3-sys", "cc", "diesel", "rusqlite", "ring", "psm"])
        cargo_body = E2_CARGO
        app_compile = "compile('stock_analysis',app/'src/lib.rs','TEST_CODE_app',deps,['--target','x86_64-apple-darwin'])"
        self.assertEqual(cargo_body.count(app_compile), 1)
        cargo_body = cargo_body.replace(app_compile, "# TEST_CODE Stage B CompileOnly ends before any application library compile/artifact.")
        if case == "source_post":
            legacy_save = "    if CASE=='source_post':saved[root/'Cargo.toml']=(root/'Cargo.toml').read_bytes()"
            self.assertEqual(cargo_body.count(legacy_save), 1)
            cargo_body = cargo_body.replace(legacy_save, "    # E2 source_post changes only C inputs; no inherited manifest restoration.")
        for name, body in (("fake-native-cc", E2_NATIVE), ("fake-cargo", cargo_body.replace("__CASE__", repr(case)))):
            path = write(self.root / name, "#!" + PYTHON + " -I\n" + body); path.chmod(0o700)
        inventory["cargo"] = {"path": str(self.root / "fake-cargo"), "sha256": sha(self.root / "fake-cargo")}
        inventory["generators"]["CC"] = {"path": str(self.root / "fake-native-cc"), "sha256": sha(self.root / "fake-native-cc")}
        body = self.tool.read_text()
        # Isolated fake source/tool pins replace only the two fixed map witnesses.
        for key, value in (("FOREIGN_MAP_RUSTC_SHA256", inventory["rustc"]["sha256"]),
                           ("FOREIGN_MAP_CC_SHA256", sha(vendor / "cc/src/command_helpers.rs"))):
            original = key + " = " + json.dumps(getattr(owner, key))
            self.assertEqual(body.count(original), 1)
            body = body.replace(original, key + " = " + json.dumps("0" * 64 if case == "template_map" else value))
        if case == "capture_fault":
            needle = 'try:dest=open(call/(name+".raw"),"xb")';self.assertEqual(body.count(needle), 1)
            body = body.replace(needle, 'try:\n                if env.get("E1_CASE")=="capture_fault" and "-c" in argv and name=="stderr":raise OSError("TEST_CODE capture")\n                dest=open(call/(name+".raw"),"xb")')
        if case == "forward_fault":
            needle = 'written = os.write(1 if stream == "stdout" else 2, view)';self.assertEqual(body.count(needle), 1)
            body = body.replace(needle, 'if compiling:raise OSError("TEST_CODE forward")\n                            ' + needle)
        if case == "fd_return":
            needle = 'receipt["tool_result"] = code; receipt["failures"].extend(faults)';self.assertEqual(body.count(needle), 1)
            body = body.replace(needle, needle + '\n        if compiling:os.close(fds[0])')
        self.tool.write_text(body); inventory["owner_sha256"] = sha(self.tool)
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly", "profile": owner.BUNDLED_PROFILE, "inventory": inventory}))
        return inventory


    def native_e2_result(self, case="normal"):
        inventory = self.prepare_native_e2(case)
        run = subprocess.run([PYTHON, "-I", str(self.tool), "record"], env=dict(os.environ),
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120)
        self.assertEqual(run.returncode, 2, run.stdout.decode(errors="replace") + run.stderr.decode(errors="replace"))
        record = self.record_result(run); session = Path(json.loads(run.stdout)["record_path"]).parent
        foreign = json.loads((session / "foreign-native-record.json").read_text())
        calls = [(p.parent, json.loads(p.read_text())) for p in (session / "foreign-native-invocations").glob("*/receipt.json")]
        self.assertEqual(record["foreign_native_record_sha256"], sha(session / "foreign-native-record.json"))
        return inventory, session, record, foreign, calls


    def native_e2_private_negative_result(self, case):
        # This fixed cut observes real negative ownership after the CLI inventory abort.
        self.assertIn(case, ("source_post", "include_post", "private_header_post",
                             "private_header_copy_retained", "private_header_copy_pre", "private_header_copy_request_only"))
        from unittest import mock
        inventory = self.prepare_native_e2(case)
        pending_root = self.root / ".replay-build-records"
        before = set(pending_root.glob("pending-*"))
        run = subprocess.run([PYTHON, "-I", str(self.tool), "record"], env=dict(os.environ),
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120)
        self.assertEqual(run.returncode, 2, run.stderr.decode(errors="replace"))
        self.assertEqual(run.stdout, b"")
        self.assertEqual(json.loads(run.stderr), {"schema": owner.SCHEMA, "state": "RecordingOnly",
                                                 "reason": "Refused", "detail": "InventoryMismatch"})
        pending = set(pending_root.glob("pending-*")) - before
        self.assertEqual(len(pending), 1);session = pending.pop()
        self.assertFalse((session / "record.json").exists())
        self.assertFalse((session / "foreign-native-record.json").exists())
        forwarded = session / "foreign-forwarded.json"
        self.assertTrue(forwarded.is_file(), (session / "cargo.stderr.raw").read_text(errors="replace"))
        self.assertEqual({row["name"] for row in json.loads(forwarded.read_bytes())}, {"ring", "psm"})
        calls = [(p.parent, json.loads(p.read_text())) for p in (session / "foreign-native-invocations").glob("*/receipt.json")]
        self.assertTrue(calls)
        with mock.patch.object(owner, "POLICY", self.policy):
            privateNegativeObservation = owner.foreign_evidence_namespace(session)
        # The real tuple is never presented as a persisted foreign record or qualification.
        return inventory, session, calls, privateNegativeObservation


    def prepare_native_e3(self, case="ar_normal"):
        inventory = self.prepare_native_e2("normal")
        cargo_path = self.root / "fake-cargo"; cargo_body = cargo_path.read_text()
        needle = "CASE='normal';argv="; self.assertEqual(cargo_body.count(needle), 1)
        cargo_body = cargo_body.replace(needle, "CASE=" + repr(case) + ";argv=")
        needle = "    selected=E2_SOURCE_DATA[name] if CASE in ('normal','parallel') else E2_SOURCE_DATA[name][:2 if CASE=='sticky' else 1]"
        self.assertEqual(cargo_body.count(needle), 1)
        cargo_body = cargo_body.replace(needle, "    selected=E3_SELECTED[name]")
        cargo_body = cargo_body.replace("import hashlib,json,os,pathlib,subprocess,sys,threading", "import hashlib,json,os,pathlib,subprocess,sys,threading\nE3_SELECTED=" + repr(E3_SELECTED))
        needle = "        for specification in selected:object_call(specification)"; self.assertEqual(cargo_body.count(needle), 1)
        cargo_body = cargo_body.replace(needle, needle + "\n" + E3_ARCHIVE_CARGO)
        # Only E3 malformed-receipt controls avoid the old E2 tail's annotation access.
        needle = "object_paths=[p for p in paths if json.loads(p.read_text()).get('operation',{}).get('class')=='CompilerObjectCompile'"
        self.assertEqual(cargo_body.count(needle), 1)
        cargo_body = cargo_body.replace(needle, "object_paths=[p for p in paths if isinstance(json.loads(p.read_text()).get('operation'),dict) and json.loads(p.read_text())['operation'].get('class')=='CompilerObjectCompile'")
        needle = "emit({'reason':'build-finished','success':True})"; self.assertEqual(cargo_body.count(needle), 1)
        cargo_body = cargo_body.replace(needle, E3_COPY_CARGO + "\n" + needle)
        cargo_path.write_text(cargo_body)
        ar_path = write(self.root / "fake-native-ar", "#!" + PYTHON + " -I\n" + E3_NATIVE_AR); ar_path.chmod(0o700)
        inventory["cargo"]["sha256"] = sha(cargo_path)
        inventory["generators"]["AR"] = {"path": str(ar_path), "sha256": sha(ar_path)}
        body = self.tool.read_text()
        if case in ("ar_order_cc_first", "ar_order_ar_first"):
            needle = 'call = namespace / uuid.uuid4().hex; call.mkdir(mode=0o700)'; self.assertEqual(body.count(needle), 1)
            # Private fixture IDs remain valid directory identities; production UUID creation is unchanged.
            prefix = '("f" if role == "ar" else "0")' if case == "ar_order_cc_first" else '("0" if role == "ar" else "f")'
            body = body.replace(needle, 'call = namespace / (' + prefix + ' + uuid.uuid4().hex[1:]); call.mkdir(mode=0o700)')
        if case == "ar_capture":
            needle = 'try:dest=open(call/(name+".raw"),"xb")'; self.assertEqual(body.count(needle), 1)
            body = body.replace(needle, 'try:\n                if env.get("E1_CASE")=="ar_capture" and argv[1:2]==["cqD"] and name=="stderr":raise OSError("TEST_CODE AR capture")\n                dest=open(call/(name+".raw"),"xb")')
        if case == "ar_forward":
            needle = 'written = os.write(1 if stream == "stdout" else 2, view)'; self.assertEqual(body.count(needle), 1)
            body = body.replace(needle, 'if archiving:raise OSError("TEST_CODE AR forward")\n                            ' + needle)
        if case == "ar_fd_return":
            needle = 'receipt["tool_result"] = code; receipt["failures"].extend(faults)'; self.assertEqual(body.count(needle), 1)
            body = body.replace(needle, needle + '\n        if archiving:os.close(fds[0])')
        self.tool.write_text(body); inventory["owner_sha256"] = sha(self.tool)
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly", "profile": owner.BUNDLED_PROFILE, "inventory": inventory}))
        return inventory


    def native_e3_result(self, case="ar_normal"):
        inventory = self.prepare_native_e3(case)
        run = subprocess.run([PYTHON, "-I", str(self.tool), "record"], env=dict(os.environ),
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120)
        self.assertEqual(run.returncode, 2, run.stdout.decode(errors="replace") + run.stderr.decode(errors="replace"))
        record = self.record_result(run); session = Path(json.loads(run.stdout)["record_path"]).parent
        self.assertEqual(record["cargo_exit_code"], 0, (session / "cargo.stderr.raw").read_text(errors="replace"))
        foreign = json.loads((session / "foreign-native-record.json").read_bytes())
        self.assertEqual(record["foreign_native_record_sha256"], sha(session / "foreign-native-record.json"))
        calls = [(p.parent, json.loads(p.read_bytes())) for p in (session / "foreign-native-invocations").glob("*/receipt.json")]
        self.assertEqual({r["name"] for r in json.loads((session / "foreign-forwarded.json").read_bytes())}, {"ring", "psm"})
        self.assertEqual((foreign["stage"], foreign["native_producer_qualification"], foreign["artifact_selection"]),
                         ("StageCIncomplete", "not_issued", "not_observed"))
        self.assertEqual(foreign["unclosed"], ["archive-chain/index", "builder-run", "consumer"])
        self.assertEqual(record["selected_library"], [])
        return inventory, session, record, foreign, calls


    def test_native_e3_observed_first_append_partial_fallback_and_order(self):
        for case in ("ar_normal", "ar_partial", "ar_order_cc_first", "ar_order_ar_first"):
            with self.subTest(case=case):
                inventory, session, record, foreign, calls = self.native_e3_result(case)
                compiles = [(c, r) for c, r in calls if r.get("operation", {}).get("class") == "CompilerObjectCompile"]
                arches = [(c, r) for c, r in calls if r["role"] == "ar"]
                self.assertEqual(len(compiles), 17); self.assertEqual(len(arches), 4 if case == "ar_partial" else 2)
                self.assertTrue(all(r["protocol_state"] == "Completed" and r["failures"] == [] for _, r in calls), foreign["blockers"])
                self.assertFalse(any(b.startswith("ForeignOperation:") for b in foreign["blockers"]), foreign["blockers"])
                hits = [json.loads(p.read_bytes()) for p in (session / "archive-entry").iterdir()]
                self.assertEqual(len(hits), len(arches))
                forwarded = json.loads((session / "foreign-forwarded.json").read_bytes())
                for call, receipt in arches:
                    name = Path(receipt["context"]["manifest"]).name; raw = [os.fsdecode(bytes.fromhex(v)) for v in receipt["args_hex"]]
                    self.assertEqual(raw[2:], [str(Path(receipt["context"]["out_dir"]) / base) for _, base in E3_SELECTED[name]])
                    self.assertEqual(receipt["tool_sha256"], inventory["generators"]["AR"]["sha256"])
                    self.assertEqual(receipt["family_pre"], receipt["family_return"])
                    self.assertEqual(receipt["archive_member_producers_pre"], receipt["archive_member_producers_return"])
                    self.assertEqual(receipt["jobserver_identity"], receipt["jobserver_return"])
                    hit = next(h for h in hits if h["argv"] == raw and h["cwd"] == receipt["context"]["manifest"])
                    self.assertTrue(hit["stdin_eof"]); self.assertEqual(len(hit["fds"]), 2)
                    self.assertEqual(hit["fds"], [v["fd"] for v in receipt["jobserver_identity"]["endpoints"]])
                    self.assertEqual(hit["inodes"], [v["inode"] for v in receipt["jobserver_identity"]["endpoints"]])
                    self.assertEqual(hit["environment"]["ZERO_AR_DATE"], "1"); self.assertNotIn("LC_ALL", hit["environment"])
                    self.assertEqual(hit["environment"]["LC_CTYPE"], "C.UTF-8")
                    observed = next(v for v in forwarded if v["name"] == name and v["args"] == raw)
                    for stream in ("stdout", "stderr"):
                        path = call / (stream + ".raw"); self.assertEqual(sha(path), receipt[stream + "_sha256"])
                        self.assertEqual(path.read_bytes(), ("TEST_CODE_archive_" + stream + "\n").encode())
                        self.assertEqual(observed[stream + "_hex"], path.read_bytes().hex())
                    self.assertEqual(observed["status"], receipt["tool_result"])
                    for pre, post, reference in zip(receipt["archive_members_pre"], receipt["archive_members_post"], receipt["archive_member_producers_pre"]):
                        self.assertEqual(pre["sha256"], post["sha256"]); self.assertEqual(pre["sha256"], reference["sha256"])
                        producer = next((c, r) for c, r in compiles if c.name == reference["operation_id"])
                        self.assertEqual(reference["request_sha256"], sha(producer[0] / "request.json"))
                        self.assertEqual(reference["receipt_sha256"], sha(producer[0] / "receipt.json"))
                        for state in (pre, post):self.assertEqual(sha(call / state["snapshot"]), state["sha256"])
                    self.assertEqual(sha(call / receipt["archive_post"]["snapshot"]), receipt["archive_post"]["sha256"])
                    self.assertIn(receipt["archive_post"]["sha256"], foreign["quarantine"]["sha256"])
                    e_id = receipt["family_pre"]["effective_e_operation_id"]
                    e = next(r for c, r in calls if c.name == e_id)
                    self.assertFalse(Path(e["operation"]["source"]).exists())
                    self.assertEqual(next(o for o in foreign["operations"] if o["operation_id"] == e_id)["input_final_state"], "RetiredAfterCcReturn")
                    if receipt["operation"]["mode"] == "cqD":
                        self.assertEqual(receipt["tool_result"], 7 if case == "ar_partial" else 0)
                        self.assertFalse(receipt["archive_pre"]["exists"]); self.assertIsNone(receipt["archive_predecessor"])
                        self.assertEqual(receipt["archive_history_pre"], [])
                    else:
                        previous = next((c, r) for c, r in arches if c.name == receipt["archive_predecessor"])
                        self.assertEqual(previous[1]["tool_result"], 7)
                        self.assertEqual(receipt["archive_pre"]["sha256"], previous[1]["archive_post"]["sha256"])
                        self.assertEqual((call / "archive-pre.raw").read_bytes(), ("TEST_CODE_partial_archive:" + name + "\n").encode())
                        self.assertEqual((call / "archive-post.raw").read_bytes(), ("TEST_CODE_partial_archive:" + name + "\nTEST_CODE_append_archive:" + name + "\n").encode())
                        self.assertEqual(receipt["archive_history_pre"], receipt["archive_history_return"])
                        self.assertEqual(receipt["archive_history_pre"], [{"operation_id": previous[0].name,
                            "request_sha256": sha(previous[0] / "request.json"), "receipt_sha256": sha(previous[0] / "receipt.json")}])
                if case.startswith("ar_order_"):
                    ar_ids = [c.name for c, _ in arches]; cc_ids = [c.name for c, r in calls if r["role"] == "cc"]
                    self.assertTrue(max(cc_ids) < min(ar_ids) if case == "ar_order_cc_first" else max(ar_ids) < min(cc_ids))


    def test_native_e3_append_admission_sticky_faults_and_malformed_evidence(self):
        rejected = {"ar_env_locale": "ForeignArchiveEnvironment", "ar_env_zero": "ForeignArchiveEnvironment",
                    "ar_fd_foreign": "NativeJobserverPair", "ar_member_order": "ForeignArchiveTemplate",
                    "ar_extra_member": "ForeignArchiveTemplate", "ar_existing": "ForeignArchiveInitial",
                    "ar_cq_without_probe": "ForeignArchivePredecessor", "ar_member_missing": "No such file"}
        for case, marker in rejected.items():
            with self.subTest(case=case):
                _, session, _, foreign, calls = self.native_e3_result(case)
                arches = [r for _, r in calls if r["role"] == "ar"]
                self.assertEqual(len(arches), 2)
                self.assertTrue(all(r["protocol_state"] == "ProtocolRefused" and r["tool_result"] is None for r in arches))
                self.assertTrue(all(any(marker in f for f in r["failures"]) for r in arches), arches)
                self.assertFalse((session / "archive-entry").exists())
                self.assertTrue(any("ForeignProtocolSticky" in b for b in foreign["blockers"]))
                self.assertTrue(all(r["compile_input_declaration"]["state"] == "DeclaredOnly" for r in arches))
        for case in ("ar_bad_operation_none", "ar_bad_operation_list"):
            with self.subTest(case=case):
                _, session, _, foreign, calls = self.native_e3_result(case)
                arches = [r for _, r in calls if r["role"] == "ar"]
                self.assertEqual(len(arches), 4); self.assertEqual(len(list((session / "archive-entry").iterdir())), 2)
                fallback = [r for r in arches if [os.fsdecode(bytes.fromhex(v)) for v in r["args_hex"]][0] == "cq"]
                self.assertEqual(len(fallback), 2)
                self.assertTrue(all((r["protocol_state"], r["tool_result"], r["failures"]) ==
                                    ("ProtocolRefused", None, ["ForeignArchiveOperationFields"]) for r in fallback), fallback)
                self.assertTrue(any("ForeignOperation:" in b for b in foreign["blockers"]))
        faults = {"ar_input_post": "ForeignArchiveInputChanged", "ar_control_post": "ForeignControlChanged",
                  "ar_capture": "ForeignCaptureSticky", "ar_forward": "ForeignForward",
                  "ar_fd_return": "InvalidJobserverDescriptors", "ar_missing": "ForeignArchiveMissing"}
        for case, marker in faults.items():
            with self.subTest(case=case):
                _, session, _, foreign, calls = self.native_e3_result(case)
                arches = [(c, r) for c, r in calls if r["role"] == "ar"]
                attempted = [(c, r) for c, r in arches if r["tool_result"] is not None]
                self.assertEqual(len(attempted), 2); self.assertEqual(len(list((session / "archive-entry").iterdir())), 2)
                self.assertTrue(all(r["protocol_state"] == "ProtocolRefused" and r["tool_result"] == 0
                                    and any(marker in f for f in r["failures"]) for _, r in attempted), attempted)
                for call, receipt in attempted:
                    self.assertEqual((call / "stdout.raw").read_bytes(), b"TEST_CODE_archive_stdout\n")
                    self.assertEqual(sha(call / "stdout.raw"), receipt["stdout_sha256"])
                    if case != "ar_capture":self.assertEqual((call / "stderr.raw").read_bytes(), b"TEST_CODE_archive_stderr\n")
                    self.assertEqual(receipt["archive_post"]["exists"], case != "ar_missing")
                    if case != "ar_missing":self.assertIn(receipt["archive_post"]["sha256"], foreign["quarantine"]["sha256"])
                    self.assertEqual(len(receipt["archive_members_post"]), len(receipt["operation"]["members"]))
                    if case == "ar_input_post":self.assertNotEqual(receipt["archive_members_pre"][0]["sha256"], receipt["archive_members_post"][0]["sha256"])
                refused_fallback = [r for _, r in arches if [os.fsdecode(bytes.fromhex(v)) for v in r["args_hex"]][0] == "cq"]
                self.assertTrue(all(r["tool_result"] is None and r["protocol_state"] == "ProtocolRefused" for r in refused_fallback))
                self.assertTrue(any("ForeignProtocolSticky" in b for b in foreign["blockers"]))
        _, session, _, foreign, calls = self.native_e3_result("ar_signal")
        arches = [(c, r) for c, r in calls if r["role"] == "ar"]
        self.assertEqual(len(arches), 2)
        for call, receipt in arches:
            self.assertEqual((receipt["protocol_state"], receipt["tool_result"], receipt["failures"]), ("Completed", -15, []))
            self.assertTrue(receipt["source_semantics"]["fallback_requested"])
            self.assertTrue(receipt["archive_post"]["exists"])
            forwarded = next(r for r in json.loads((session / "foreign-forwarded.json").read_bytes())
                             if r["args"] == [os.fsdecode(bytes.fromhex(v)) for v in receipt["args_hex"]])
            self.assertEqual(forwarded["status"], 143)
        self.assertTrue(any(b.startswith("ForeignArchiveUnresolvedNonzero:") for b in foreign["blockers"]))


    def test_native_e3_archive_copy_quarantine_and_unobserved_followups(self):
        for case in ("ar_copy_source", "ar_copy_extern", "ar_copy_output", "ar_copy_retained", "ar_copy_request_only"):
            with self.subTest(case=case):
                _, session, record, foreign, calls = self.native_e3_result(case)
                control = json.loads((session / "archive-copy-control.json").read_bytes()); copy = Path(control["copy"])
                self.assertEqual(copy.read_bytes().hex(), control["body_hex"]); self.assertEqual(sha(copy), control["retained_sha256"])
                self.assertIn(sha(copy), foreign["quarantine"]["sha256"])
                cc_path = session / "invocations" / control["cc_id"] / "receipt.json"
                cc = json.loads(cc_path.read_bytes()); self.assertEqual(cc["source"], str(session / "vendor/cc/src/lib.rs"))
                self.assertEqual(sha(cc_path), control["cc_receipt_sha256"])
                bound = [r for r in record["invocations"] if r["invocation_id"] == control["cc_id"]]
                self.assertEqual(len(bound), 1); self.assertEqual(bound[0]["receipt_sha256"], sha(cc_path))
                self.assertIn("NativeOutputRole:" + (control["cc_id"] if case == "ar_copy_output" else str(copy)), record["blockers"])
                self.assertFalse(any(c["path"] == str(copy) for c in record["consumed_sources"]))
                self.assertFalse(any(e["path"] == str(copy) and e["producers"] for e in record["extern_edges"]))
                literal = str(session / "vendor" / owner.PROBE_LITERAL)
                if case == "ar_copy_output":
                    self.assertIn({"path": str(copy), "kind": "link"}, cc["declared_outputs"])
                    dep = next(o for o in cc["outputs"] if o["kind"] == "dep-info")
                    self.assertIn(literal, dep["dep_info"]["paths"]); self.assertEqual(sha(Path(dep["path"])), dep["sha256"])
                    self.assertIn(os.fsencode(literal), Path(dep["path"]).read_bytes())
                    self.assertEqual(Path(literal).stat().st_size, 206); self.assertEqual(sha(Path(literal)), owner.PROBE_DIGEST)
                    self.assertIn("UnresolvedCargoArtifact:" + cc["source"], record["blockers"])
                    self.assertFalse(any(c["path"] == literal for c in record["consumed_sources"]))
                    self.assertFalse(any(control["cc_id"] in e["producers"] for e in record["extern_edges"]))
                    self.assertFalse(any(a["producer_invocation"] == control["cc_id"] for a in record["build_script_associations"]))
                else:self.assertTrue(any(c["path"] == literal for c in record["consumed_sources"]))
                if case == "ar_copy_retained":
                    self.assertNotIn(sha(copy), {sha(p) for p in (session / "foreign-native-invocations").glob("*/*.raw")})
                    retained = [r for _, r in calls if r.get("archive_post", {}).get("sha256") == sha(copy)]
                    self.assertEqual(len(retained), 1)
                    self.assertTrue(all(not Path(r["archive_post"]["path"]).exists() for r in retained))
                    self.assertTrue(any(b.startswith("ForeignNamespace:") for b in foreign["blockers"]))
                if case == "ar_copy_request_only":
                    call = session / "foreign-native-invocations" / control["operation_id"]
                    self.assertTrue((call / "request.json").is_file()); self.assertFalse((call / "receipt.json").exists())
                    raw = [os.fsdecode(bytes.fromhex(v)) for v in json.loads((call / "request.json").read_bytes())["args_hex"]]
                    self.assertTrue(Path(raw[1]).is_file()); self.assertIn(str(Path(raw[1])), foreign["quarantine"]["paths"])
                    self.assertEqual(sha(Path(raw[1])), sha(copy))
                    self.assertNotIn(sha(copy), {sha(p) for p in (session / "foreign-native-invocations").glob("*/*.raw")})
                    self.assertFalse(any(r.get("archive_post", {}).get("sha256") == sha(copy) for _, r in calls))
                    self.assertTrue(any(b.startswith("ForeignOperation:" + call.name + ":") for b in foreign["blockers"]))
                    self.assertEqual([o.get("protocol_state") for o in foreign["operations"] if o["operation_id"] == call.name], [None])
        for case in ("ar_index", "ar_test", "ar_remaining"):
            with self.subTest(case=case):
                _, session, _, foreign, calls = self.native_e3_result(case)
                arches = [r for _, r in calls if r["role"] == "ar"]
                self.assertEqual(len(arches), 4); self.assertEqual(len(list((session / "archive-entry").iterdir())), 2)
                refused = [r for r in arches if r["protocol_state"] == "ProtocolRefused"]
                self.assertEqual(len(refused), 2)
                self.assertTrue(all(r["tool_result"] is None and r["failures"] == ["ForeignArchiveTemplate"] for r in refused), refused)
                self.assertTrue(any("ForeignProtocolSticky" in b for b in foreign["blockers"]))


    def test_native_e2_fixed_thirty_sources_and_parallel_capture(self):
        for case in ("normal", "parallel"):
            with self.subTest(case=case):
                _, session, record, foreign, calls = self.native_e2_result(case)
                compiles = [(c, r) for c, r in calls if r.get("operation", {}).get("class") == "CompilerObjectCompile"]
                self.assertEqual(len(compiles), 30);self.assertEqual(len(calls), 36)
                self.assertTrue(all(r["protocol_state"] == "Completed" and r["tool_result"] == 0 for _, r in compiles), foreign["blockers"])
                self.assertEqual({Path(r["context"]["manifest"]).name for _, r in compiles}, {"ring", "psm"})
                forwarded = json.loads((session / "foreign-forwarded.json").read_text())
                for call, receipt in compiles:
                    name = Path(receipt["context"]["manifest"]).name;raw = receipt["operation"]["raw_source"]
                    member = str(Path(raw).relative_to(session / "vendor/ring")) if name == "ring" else raw
                    expected = dict(E2_SOURCE_DATA[name])[member]
                    self.assertEqual(Path(receipt["operation"]["output"]).name, expected)
                    self.assertEqual(receipt["input_pre"]["sha256"], receipt["input_post"]["sha256"])
                    self.assertFalse(receipt["output_pre"]["exists"]);self.assertTrue(receipt["output_post"]["exists"])
                    self.assertEqual((call / "output-post.raw").read_bytes(), Path(receipt["operation"]["output"]).read_bytes())
                    self.assertEqual(receipt["jobserver_identity"], receipt["jobserver_return"])
                    self.assertEqual(receipt["family_pre"], receipt["family_return"])
                    e_id = receipt["family_pre"]["effective_e_operation_id"]
                    e_call, e_receipt = next((c, r) for c, r in calls if c.name == e_id)
                    self.assertFalse(Path(e_receipt["operation"]["source"]).exists())
                    self.assertEqual(next(o for o in foreign["operations"] if o["operation_id"] == e_id)["input_final_state"], "RetiredAfterCcReturn")
                    evidence = receipt["family_pre"]["evidence"]
                    self.assertEqual({k: len(v) for k, v in evidence.items()}, {"CompilerFamilyFileProbe": 1, "CompilerFamilyHelpProbe": 1, "CompilerFamilyVersionProbe": 1})
                    for references in evidence.values():
                        for reference in references:
                            directory = session / "foreign-native-invocations" / reference["operation_id"]
                            for leaf in ("request", "receipt", "stdout", "stderr"):
                                self.assertEqual(reference[leaf + "_sha256"], sha(directory / (leaf + (".json" if leaf in ("request", "receipt") else ".raw"))))
                    observed = next(v for v in forwarded if v["args"] == [os.fsdecode(bytes.fromhex(a)) for a in receipt["args_hex"]])
                    self.assertEqual(observed["stdout_hex"], (call / "stdout.raw").read_bytes().hex())
                    self.assertEqual(observed["stderr_hex"], (call / "stderr.raw").read_bytes().hex())
                    self.assertIn(receipt["output_post"]["sha256"], foreign["quarantine"]["sha256"])
                    if name == "ring":
                        for member in E2_PRIVATE_HEADER_MEMBERS:
                            path = session / "vendor" / member;digest = sha(path)
                            self.assertTrue(all(receipt[k]["source_sha256"][member] == digest for k in ("compile_pins_pre", "compile_pins_post", "compile_pins_return")))
                            self.assertIn(str(path), foreign["quarantine"]["paths"])
                            self.assertIn(digest, foreign["quarantine"]["sha256"])
                self.assertEqual(foreign["stage"], "StageBIncomplete")
                self.assertEqual(foreign["unclosed"], ["archive", "builder-run", "consumer"])
                self.assertFalse(any("ForeignOperation:" in v for v in foreign["blockers"]), foreign["blockers"])
                self.assertEqual((foreign["native_producer_qualification"], foreign["artifact_selection"]), ("not_issued", "not_observed"))
                self.assertEqual(record["selected_library"], [])


    def test_native_e2_template_locale_pins_and_genuine_family_rejections(self):
        cases = {"template_source": "ForeignCompileSource", "template_flag": "ForeignCompileArgv",
                 "template_order": "ForeignCompileArgv", "template_prefix": "ForeignCompileArgv",
                 "template_suffix": "ForeignCompileArgv", "template_escape": "ForeignCompileArgv",
                 "template_locale": "ForeignCompileEnvironment", "template_hash": "ForeignCompileSourcePin",
                 "template_helper": "ForeignCompileSourcePin", "template_map": "ForeignObjectMapBinding",
                 "template_fd": "Jobserver", "template_existing": "ForeignCompileOwnership",
                 "family_missing": "ForeignFamilyUnique", "family_duplicate": "ForeignFamilyUnique",
                 "family_msvc": "ForeignFamilyClang", "family_zig": "ForeignFamilyClang",
                 "family_stream": "ForeignFamilyStream", "family_label": "ForeignFamilySemantics",
                 "family_control": "ForeignFamilyControl", "family_fd": "ForeignJobserverChanged",
                 "family_resurrect": "NativeVersionChanged", "family_pending": "ForeignFamilyPending"}
        for case, reason in cases.items():
            with self.subTest(case=case):
                _, session, _, foreign, calls = self.native_e2_result(case)
                compiles = [(c, r) for c, r in calls if "-c" in [os.fsdecode(bytes.fromhex(a)) for a in r["args_hex"]]]
                self.assertEqual(len(compiles), 2)
                self.assertTrue(all(r["protocol_state"] == "ProtocolRefused" and r["tool_result"] is None for _, r in compiles))
                self.assertTrue(all(any(reason in f for f in r["failures"]) for _, r in compiles), [r["failures"] for _, r in compiles])
                hits = [json.loads(p.read_text()) for p in (session / "foreign-entry").iterdir()]
                self.assertFalse(any("-c" in h["argv"] for h in hits))
                self.assertTrue(any("ForeignProtocolSticky" in b for b in foreign["blockers"]))


        _, _, _, _, calls = self.native_e2_result("template_include")
        ring = [r for _, r in calls if "-c" in [os.fsdecode(bytes.fromhex(a)) for a in r["args_hex"]] and r["context"]["package_id"] == owner.RING_PACKAGE]
        self.assertEqual(len(ring), 1);self.assertEqual(ring[0]["tool_result"], None)
        self.assertIn("ForeignCompileSourcePin", ring[0]["failures"])


        _, session, _, _, calls = self.native_e2_result("private_header_pre")
        ring = [(c, r) for c, r in calls if "-c" in [os.fsdecode(bytes.fromhex(a)) for a in r["args_hex"]] and r["context"]["package_id"] == owner.RING_PACKAGE]
        self.assertEqual(len(ring), 1)
        self.assertEqual((ring[0][1]["protocol_state"], ring[0][1]["tool_result"], ring[0][1]["failures"]), ("ProtocolRefused", None, ["ForeignCompileSourcePin"]))
        hits = [json.loads(p.read_text()) for p in (session / "foreign-entry").iterdir()]
        self.assertFalse(any("-c" in h["argv"] and h["cwd"] == str(session / "vendor/ring") for h in hits))


    def test_native_e2_signed_status_partial_objects_and_sticky_faults(self):
        cases = {"nonzero": (7, "ForeignCompileNonzeroOrMissing"), "signal": (-15, "ForeignCompileNonzeroOrMissing"),
                 "object_missing": (0, "ForeignCompileObjectMissing"), "source_post": (0, "ForeignInputChanged"),
                 "capture_fault": (0, "ForeignCaptureSticky"), "forward_fault": (0, "ForeignForward"), "fd_return": (0, "InvalidJobserverDescriptors")}
        for case, (status, reason) in cases.items():
            with self.subTest(case=case):
                if case == "source_post":
                    _, session, calls, privateNegativeObservation = self.native_e2_private_negative_result(case)
                    paths, _, hashes, _ = privateNegativeObservation
                    compiles = [(c, r) for c, r in calls if r.get("operation", {}).get("class") == "CompilerObjectCompile"]
                    self.assertEqual(len(compiles), 2)
                    for call, receipt in compiles:
                        self.assertEqual((receipt["protocol_state"], receipt["tool_result"], receipt["failures"]),
                                         ("ProtocolRefused", 0, ["ForeignInputChanged"]))
                        request = json.loads((call / "request.json").read_bytes())
                        self.assertEqual(request["args_hex"], receipt["args_hex"])
                        for stream in ("stdout", "stderr"):
                            self.assertEqual((call / (stream + ".raw")).read_bytes(), ("TEST_CODE_compile_" + stream + "\n").encode())
                            self.assertEqual(sha(call / (stream + ".raw")), receipt[stream + "_sha256"])
                        for key in ("input_pre", "input_post", "output_post"):
                            state = receipt[key];snapshot_path = call / state["snapshot"]
                            self.assertTrue(state["exists"]);self.assertEqual(sha(snapshot_path), state["sha256"])
                            self.assertIn(state["sha256"], hashes)
                            copy = session / ("target/source-post-" + call.name + "-" + key + ".bin");copy.write_bytes(snapshot_path.read_bytes())
                            self.assertNotIn(str(copy), paths)
                            self.assertTrue(owner.foreign_owned(str(copy), session, paths, hashes))
                        source = Path(receipt["operation"]["source"])
                        self.assertEqual(source.read_bytes(), b"X")
                        self.assertEqual(sha(source), receipt["input_post"]["sha256"])
                        self.assertNotEqual(receipt["input_pre"]["sha256"], receipt["input_post"]["sha256"])
                        self.assertIn(str(source), paths)
                        self.assertEqual(sha(Path(receipt["operation"]["output"])), receipt["output_post"]["sha256"])
                        self.assertNotIn("compile_pins_post", receipt)
                    continue
                _, session, _, foreign, calls = self.native_e2_result(case)
                compiles = [(c, r) for c, r in calls if r.get("operation", {}).get("class") == "CompilerObjectCompile"]
                self.assertEqual(len(compiles), 2)
                for call, receipt in compiles:
                    self.assertEqual(receipt["tool_result"], status)
                    self.assertIn(b"TEST_CODE_compile_stdout", (call / "stdout.raw").read_bytes())
                    if case != "capture_fault":self.assertIn(b"TEST_CODE_compile_stderr", (call / "stderr.raw").read_bytes())
                    self.assertTrue(any(reason in b for b in [*foreign["blockers"], *receipt["failures"]]), [foreign["blockers"], receipt["failures"]])
                    if case != "object_missing":
                        self.assertTrue(receipt["output_post"]["exists"])
                        self.assertIn(receipt["output_post"]["sha256"], foreign["quarantine"]["sha256"])
                if status == 0:self.assertTrue(all(r["protocol_state"] == "ProtocolRefused" and r["failures"] for _, r in compiles))
        _, _, _, foreign, calls = self.native_e2_result("sticky")
        ring = [r for _, r in calls if r.get("operation", {}).get("class") == "CompilerObjectCompile" and r["context"]["package_id"] == owner.RING_PACKAGE]
        self.assertEqual(len(ring), 2)
        self.assertEqual(sum(r["tool_result"] == 7 for r in ring), 1)
        self.assertEqual(sum(r["tool_result"] is None for r in ring), 1)
        self.assertTrue(any("ForeignFamilySticky" in f for r in ring for f in r["failures"]))


        for case, member in (("include_post", "ring/include/TEST_CODE.h"),
                             ("private_header_post", "ring/crypto/internal.h")):
            with self.subTest(case=case):
                inventory, session, calls, privateNegativeObservation = self.native_e2_private_negative_result(case)
                paths, _, hashes, _ = privateNegativeObservation
                ring = [(c, r) for c, r in calls if r.get("operation", {}).get("class") == "CompilerObjectCompile" and r["context"]["package_id"] == owner.RING_PACKAGE]
                self.assertEqual(len(ring), 1);call, receipt = ring[0]
                self.assertEqual((receipt["protocol_state"], receipt["tool_result"], receipt["failures"]), ("ProtocolRefused", 0, ["ForeignCompileSourcePin"]))
                request = json.loads((call / "request.json").read_bytes())
                self.assertEqual(request["args_hex"], receipt["args_hex"])
                for stream in ("stdout", "stderr"):
                    self.assertEqual((call / (stream + ".raw")).read_bytes(), ("TEST_CODE_compile_" + stream + "\n").encode())
                    self.assertEqual(sha(call / (stream + ".raw")), receipt[stream + "_sha256"])
                for key in ("input_pre", "input_post", "output_post"):
                    state = receipt[key];self.assertTrue(state["exists"])
                    self.assertEqual(sha(call / state["snapshot"]), state["sha256"])
                    self.assertIn(state["sha256"], hashes)
                self.assertEqual(receipt["input_pre"]["sha256"], receipt["input_post"]["sha256"])
                self.assertEqual(sha(Path(receipt["operation"]["output"])), receipt["output_post"]["sha256"])
                self.assertNotIn("compile_pins_post", receipt)
                self.assertNotIn("compile_pins_return", receipt)
                header = session / "vendor" / member;expected = inventory["vendor"]["files"][member]
                self.assertEqual(receipt["compile_pins_pre"]["source_sha256"][member], expected)
                self.assertEqual(receipt["compile_input_declaration"]["source_sha256"][member], expected)
                self.assertNotEqual(sha(header), expected)
                self.assertEqual(header.read_bytes(), b"X" if case == "include_post" else b"TEST_CODE_private_header_post")
                self.assertIn(str(header), paths)
                self.assertIn(sha(header), hashes);self.assertIn(expected, hashes)
                original = self.root / "vendor-origin" / member;self.assertEqual(sha(original), expected)
                for kind, body in (("current", header.read_bytes()), ("retained", original.read_bytes()),
                                   ("object", (call / receipt["output_post"]["snapshot"]).read_bytes())):
                    copy = session / ("target/" + case + "-" + kind + ".bin");copy.write_bytes(body)
                    self.assertNotIn(str(copy), paths)
                    self.assertTrue(owner.foreign_owned(str(copy), session, paths, hashes))
                hits = [json.loads(p.read_text()) for p in (session / "foreign-entry").iterdir()]
                self.assertEqual(sum("-c" in h["argv"] and h["cwd"] == str(session / "vendor/ring") for h in hits), 1)


    def test_native_e2_quarantine_retained_objects_seal_and_archive_refusal(self):
        for case in ("object_mutated", "object_snapshot_missing", "object_request_only", "template_duplicate", "archive",
                     "ownership_object_source", "ownership_object_extern", "ownership_object_output", "ownership_object_retained"):
            with self.subTest(case=case):
                _, session, record, foreign, calls = self.native_e2_result(case)
                self.assertEqual(record["selected_library"], [])
                self.assertEqual((foreign["native_producer_qualification"], foreign["artifact_selection"]), ("not_issued", "not_observed"))
                if case.startswith("ownership_object_"):
                    copy = session / "target/copied-object.bin";self.assertTrue(copy.is_file())
                    self.assertIn(sha(copy), foreign["quarantine"]["sha256"])
                    self.assertTrue(any("NativeOutputRole" in b for b in record["blockers"]), record["blockers"])
                    self.assertFalse(any(c["path"] == str(copy) for c in record["consumed_sources"]))
                    self.assertFalse(any(e["path"] == str(copy) and e["producers"] for e in record["extern_edges"]))
                    if case.endswith("retained"):
                        self.assertNotIn(sha(copy), {sha(p) for p in (session / "foreign-native-invocations").glob("*/*.raw")})
                        self.assertTrue(any("ForeignNamespace" in b for b in foreign["blockers"]))
                elif case == "archive":
                    ar = [r for _, r in calls if r["role"] == "ar"]
                    self.assertEqual(len(ar), 2);self.assertTrue(all(r["protocol_state"] == "ProtocolRefused" and r["tool_result"] is None for r in ar))
                    self.assertFalse(any(h["argv"][:1] == ["cqD"] for h in [json.loads(p.read_text()) for p in (session / "foreign-entry").iterdir()]))
                else:self.assertTrue(any("ForeignOperation:" in b for b in foreign["blockers"]))
                literal = str(session / "vendor" / owner.PROBE_LITERAL)
                if case == "ownership_object_output":
                    # A denied declared output rejects the whole cc call before its literal edge.
                    cc_source = str(session / "vendor/cc/src/lib.rs")
                    cc_paths = [p for p in (session / "invocations").glob("*/receipt.json") if json.loads(p.read_text()).get("source") == cc_source]
                    self.assertEqual(len(cc_paths), 1);cc_path = cc_paths[0]
                    cc = json.loads(cc_path.read_bytes());call_id = cc_path.parent.name
                    bound = [r for r in record["invocations"] if r["invocation_id"] == call_id]
                    self.assertEqual(len(bound), 1);self.assertEqual(bound[0]["receipt_sha256"], sha(cc_path))
                    self.assertIn({"path": str(copy), "kind": "link"}, cc["declared_outputs"])
                    dep = [o for o in cc["outputs"] if o["kind"] == "dep-info"]
                    self.assertEqual(len(dep), 1);self.assertIn(literal, dep[0]["dep_info"]["paths"])
                    dep_path = Path(dep[0]["path"]);self.assertEqual(sha(dep_path), dep[0]["sha256"])
                    self.assertIn(os.fsencode(literal), dep_path.read_bytes())
                    self.assertEqual(Path(literal).stat().st_size, 206);self.assertEqual(sha(Path(literal)), owner.PROBE_DIGEST)
                    self.assertIn("NativeOutputRole:" + call_id, record["blockers"])
                    self.assertIn("UnresolvedCargoArtifact:" + cc_source, record["blockers"])
                    self.assertFalse(any(c["path"] == literal for c in record["consumed_sources"]))
                    self.assertFalse(any(call_id in e["producers"] for e in record["extern_edges"]))
                    self.assertFalse(any(a["producer_invocation"] == call_id for a in record["build_script_associations"]))
                    self.assertFalse(any(a["invocation_id"] == call_id for a in record["selected_library"]))
                else:
                    self.assertTrue(any(c["path"] == literal for c in record["consumed_sources"]))


        for case in ("private_header_copy_current", "private_header_copy_retained", "private_header_copy_pre", "private_header_copy_request_only"):
            with self.subTest(case=case):
                if case != "private_header_copy_current":
                    inventory, session, calls, privateNegativeObservation = self.native_e2_private_negative_result(case)
                    paths, _, hashes, blockers = privateNegativeObservation
                    control = json.loads((session / "private-header-copy-control.json").read_text())
                    copy = Path(control["copy"]);source = Path(control["source"]);retained = control["retained_sha256"]
                    self.assertEqual(copy.read_bytes().hex(), control["body_hex"]);self.assertEqual(sha(copy), retained)
                    self.assertNotIn(str(copy), paths);self.assertIn(str(source), paths)
                    self.assertIn(retained, hashes);self.assertIn(sha(source), hashes)
                    self.assertTrue(owner.foreign_owned(str(copy), session, paths, hashes))
                    self.assertEqual(control["expected_sha256"], inventory["vendor"]["files"][control["member"]])
                    cc_paths = [p for p in (session / "invocations").glob("*/receipt.json") if json.loads(p.read_text()).get("source") == str(session / "vendor/cc/src/lib.rs")]
                    self.assertEqual(len(cc_paths), 1);cc = json.loads(cc_paths[0].read_text())
                    if case in ("private_header_copy_retained", "private_header_copy_request_only"):
                        self.assertEqual(control["role"], "extern")
                        self.assertEqual([e["path"] for e in cc["externs"] if e["name"] == "private_header_copy"], [str(copy)])
                        self.assertNotEqual(sha(source), retained)
                        self.assertNotIn(retained, {sha(p) for p in (session / "foreign-native-invocations").glob("*/*.raw")})
                        self.assertNotIn(retained, {sha(session / "vendor" / member) for member in E2_PRIVATE_HEADER_MEMBERS})
                    else:
                        self.assertEqual(control["role"], "source")
                        self.assertTrue(any(str(copy) in o["dep_info"]["paths"] for o in cc["outputs"] if o["kind"] == "dep-info"))
                    call = session / "foreign-native-invocations" / control["operation_id"]
                    if case == "private_header_copy_request_only":
                        self.assertFalse((call / "receipt.json").exists())
                        self.assertFalse(any(r.get("context", {}).get("package_id") == owner.RING_PACKAGE and r.get("operation", {}).get("class") == "CompilerObjectCompile" for _, r in calls))
                        request = json.loads((call / "request.json").read_bytes())
                        self.assertEqual((request["schema"], request["state"], request["lane"], request["role"]), (owner.NATIVE_SCHEMA, "RecordingOnly", owner.FOREIGN_LANE, "cc"))
                        raw = [os.fsdecode(bytes.fromhex(v)) for v in request["args_hex"]]
                        self.assertEqual(os.fsdecode(bytes.fromhex(request["cwd_hex"])), str(session / "vendor/ring"))
                        self.assertEqual(raw[-2:], ["-c", str(session / "vendor/ring" / E2_SOURCE_DATA["ring"][0][0])])
                        output = Path(raw[raw.index("-o") + 1])
                        self.assertTrue(output.is_file());self.assertIn(str(output), paths);self.assertIn(sha(output), hashes)
                        # Receipt deletion removes status authority; remaining raw bytes still own copies negatively.
                        for stream in ("stdout", "stderr"):
                            raw_path = call / (stream + ".raw")
                            self.assertEqual(raw_path.read_bytes(), ("TEST_CODE_compile_" + stream + "\n").encode())
                            self.assertIn(sha(raw_path), hashes)
                    else:
                        receipt = next(r for c, r in calls if c == call)
                        self.assertEqual(receipt["compile_input_declaration"]["state"], "DeclaredOnly")
                        self.assertEqual(receipt["compile_input_declaration"]["source_sha256"][control["member"]], control["expected_sha256"])
                        with self.assertRaises(json.JSONDecodeError):json.loads((call / "request.json").read_bytes())
                        self.assertTrue(any(b.startswith("ForeignNamespace:" + call.name + ":") for b in blockers), blockers)
                        if case == "private_header_copy_pre":
                            self.assertEqual((receipt["protocol_state"], receipt["tool_result"], receipt["failures"]), ("ProtocolRefused", None, ["ForeignCompileSourcePin"]))
                            self.assertNotIn("compile_pins_pre", receipt);self.assertNotIn("output_post", receipt)
                            self.assertNotEqual(control["expected_sha256"], retained)
                            self.assertIn(control["expected_sha256"], hashes)
                            hits = [json.loads(p.read_text()) for p in (session / "foreign-entry").iterdir()]
                            self.assertFalse(any("-c" in h["argv"] and h["cwd"] == str(session / "vendor/ring") for h in hits))
                        else:
                            self.assertEqual((receipt["protocol_state"], receipt["tool_result"], receipt["failures"]), ("Completed", 0, []))
                            self.assertTrue(all(receipt[k]["source_sha256"][control["member"]] == retained for k in ("compile_pins_pre", "compile_pins_post", "compile_pins_return")))
                            for stream in ("stdout", "stderr"):
                                raw_path = call / (stream + ".raw")
                                self.assertEqual(raw_path.read_bytes(), ("TEST_CODE_compile_" + stream + "\n").encode())
                                self.assertEqual(sha(raw_path), receipt[stream + "_sha256"])
                            self.assertTrue(receipt["output_post"]["exists"])
                            self.assertEqual(sha(call / receipt["output_post"]["snapshot"]), receipt["output_post"]["sha256"])
                            self.assertIn(receipt["output_post"]["sha256"], hashes)
                    continue
                _, session, record, foreign, calls = self.native_e2_result(case)
                control = json.loads((session / "private-header-copy-control.json").read_text())
                copy = Path(control["copy"]);source = Path(control["source"]);retained = control["retained_sha256"]
                self.assertEqual(copy.read_bytes().hex(), control["body_hex"]);self.assertEqual(sha(copy), retained)
                self.assertNotIn(str(copy), foreign["quarantine"]["paths"])
                self.assertIn(str(source), foreign["quarantine"]["paths"])
                self.assertIn(retained, foreign["quarantine"]["sha256"])
                self.assertIn("NativeOutputRole:" + str(copy), record["blockers"])
                self.assertFalse(any(c["path"] == str(copy) for c in record["consumed_sources"]))
                self.assertFalse(any(e["path"] == str(copy) and e["producers"] for e in record["extern_edges"]))
                self.assertEqual(record["selected_library"], [])
                literal = str(session / "vendor" / owner.PROBE_LITERAL)
                self.assertTrue(any(c["path"] == literal for c in record["consumed_sources"]))


    def test_finite_argument_and_dep_info_parsers(self):
        for args in [["@response"], ["--sysroot", "/a", "--sysroot=/b"], ["-Zrandomize-layout"]]:
            with self.assertRaises(owner.Refusal):
                owner.parse_rustc(args)
        parsed = owner.parse_rustc(["--crate-name=x", "--emit=dep-info,metadata", "--out-dir", "/tmp/out", "-Cmetadata=one", "src/lib.rs"])
        self.assertEqual(parsed["codegen"]["metadata"], ["one"])
        dep = owner.dep_info(b"out: /tmp/a\\ b.rs \\\n /tmp/c.rs\n# env-dep:OUT_DIR=/tmp/out\n")
        self.assertEqual(dep["paths"], ["/tmp/a b.rs", "/tmp/c.rs"])
        self.assertEqual(len(dep["environment_comment_hex"]), 1)
        with self.assertRaises(owner.Refusal):
            owner.dep_info(b"out: $(shell unsafe)\n")


    def record10_result(self, case="probe0-both", status=0):
        inventory = self.prepare("nested:normal")
        vendor = self.root / "vendor-origin"
        for name, version, sources in (("rustversion", "1.0.22", ("build/build.rs", "build/rustc.rs", "src/lib.rs")),
                                        ("thiserror", "2.0.18", ("build.rs", "build/probe.rs", "src/lib.rs"))):
            write(vendor / name / "Cargo.toml", '[package]\nname="' + name + '"\nversion="' + version + '"\n')
            for source in sources:
                write(vendor / name / source, "// TEST_CODE fixed protocol source " + source + "\n")
            write(vendor / name / ".cargo-checksum.json", '{"files":{},"package":"TEST_CODE"}')
            inventory["packages"].append({"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
                                           "tree": "vendor", "manifest": name + "/Cargo.toml"})
        inventory["vendor"] = snapshot(vendor, ["dep", "libc", "proc-macro2", "rustversion", "thiserror"])
        if case.endswith(":inventory"):
            name = case.split(":")[1]
            next(p for p in inventory["packages"] if p["manifest"] == name + "/Cargo.toml")["id"] = "TEST_CODE_wrong_package_identity"
        rustc = write(self.root / "fake-rustc", "#!" + PYTHON + " -I\n" + RECORD10_RUSTC)
        cargo = write(self.root / "fake-cargo", "#!" + PYTHON + " -I\n" + RECORD10_CARGO.replace("__CASE__", repr(case)))
        rustc.chmod(0o700); cargo.chmod(0o700)
        inventory["rustc"] = {"path": str(rustc), "sha256": sha(rustc)}
        inventory["cargo"] = {"path": str(cargo), "sha256": sha(cargo)}
        inventory["generators"]["PROTOC"] = dict(inventory["rustc"])
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly", "profile": owner.PROFILE, "inventory": inventory}))
        result = self.invoke("record")
        self.assertEqual(result.returncode, status, result.stderr.decode(errors="replace"))
        record = self.record_result(result)
        session = Path(json.loads(result.stdout)["record_path"]).parent
        receipts = [(p.parent, json.loads(p.read_text())) for p in (session / "invocations").glob("*/receipt.json")]
        return session, record, receipts

    def test_record10_nested_origins_preserve_actual_raw_requests(self):
        session, record, receipts = self.record10_result()
        self.assertEqual(record["blockers"], [])
        self.assertEqual(len(record["selected_library"]), 1)
        children = [(p, r) for p, r in receipts if r["context"]["kind"] in owner.RECORD10_KINDS]
        self.assertEqual(len(children), 2)
        for path, r in children:
            name = "rustversion" if r["kind"] == "Probe" else "thiserror"
            attempt = json.loads((session / (name + "-attempt.json")).read_text())
            request = json.loads((path / "request.json").read_text())
            self.assertEqual(request["argv_hex"], attempt["argv_hex"])
            self.assertEqual(request["environment_hex"], attempt["environment_hex"])
            hit = json.loads((session / "compiler-entry" / (("version-" if name == "rustversion" else "feature-probe-") + name + ".json")).read_text())
            self.assertEqual([os.fsencode(a).hex() for a in hit["argv"]], request["argv_hex"])
            self.assertEqual({os.fsencode(k).hex(): os.fsencode(v).hex() for k, v in hit["environment"].items()}, request["environment_hex"])
            self.assertEqual((path / "stdout.raw").read_bytes().hex(), hit["stdout_hex"])
            self.assertEqual((path / "stderr.raw").read_bytes().hex(), hit["stderr_hex"])
            self.assertEqual(len(request["argv_hex"]), 2 if name == "rustversion" else 11)
            self.assertEqual(r["externs"], [])
            self.assertNotIn("stdin", r)
        self.assertEqual({r["role"] for _, r in receipts if r["source"] and r["source"].endswith("src/lib.rs")}, {"Host", "Target"})
        for _, r in receipts:
            if r["source"] and (r["source"].endswith("build.rs") or r["source"].endswith("src/lib.rs")):
                self.assertEqual(r["context"]["kind"], "DirectCargoCompile")
        _, ordinary_record, ordinary_receipts = self.record10_result("ordinary-probe")
        ordinary = [r for _, r in ordinary_receipts if r["context"]["kind"] == "DirectCargoProbe"]
        self.assertEqual(len(ordinary), 1)
        self.assertEqual(ordinary[0]["exit_code"], 0)
        self.assertEqual(ordinary_record["blockers"], [])

    def test_record10_nested_origin_identity_refuses_before_compiler(self):
        for name in ("rustversion", "thiserror"):
            for case in ("inventory", "version", "components", "package", "manifest", "cwd", "features", "host", "target-env", "outdir", "wrapper", "compiler", "loader", "bootstrap", "stage", "workspace", "workspace-empty", "encoded", "source", "symlink", "hardlink", "directory", "manifest-hash") + (("feature-missing",) if name == "thiserror" else ()):
                with self.subTest(name=name, case=case):
                    session, record, _ = self.record10_result("reject:" + name + ":" + case, 2)
                    self.assertFalse((session / "compiler-entry" / (("version-" if name == "rustversion" else "feature-probe-") + name + ".json")).exists())
                    rejected = [p for p in (session / "invocations").glob("*/request.json") if not (p.parent / "receipt.json").exists()]
                    self.assertEqual(len(rejected), 1)
                    self.assertFalse((rejected[0].parent / "invocation.json").exists())
                    self.assertTrue(any(b.startswith("IncompleteInvocation:") for b in record["blockers"]))

    def test_record10_closed_query_probe_templates_do_not_fall_back(self):
        for name in ("rustversion", "thiserror"):
            for case in ("empty", "retry", "extra", "native", "extern") + (("missing-group", "reorder", "wrong-source", "missing-target") if name == "thiserror" else ()):
                with self.subTest(name=name, case=case):
                    session, record, _ = self.record10_result("reject:" + name + ":" + case, 2)
                    self.assertFalse((session / "compiler-entry" / (("version-" if name == "rustversion" else "feature-probe-") + name + ".json")).exists())
                    self.assertTrue(any(b.startswith("IncompleteInvocation:") for b in record["blockers"]))
        self.assertEqual(self.nested_result()[1]["blockers"], [])
        self.assertEqual(self.ring9_result()[1]["blockers"], [])
        self.assertEqual(self.fix5_result("framework")[1]["blockers"], [])

    def test_record10_transient_probe_retains_real_failure_and_cleanup(self):
        for case, code, count in (("probe0-both", 0, 2), ("probe1-none", 1, 0), ("probe1-dep", 1, 1), ("probe1-meta", 1, 1), ("probe1-both", 1, 2)):
            with self.subTest(case=case):
                session, record, receipts = self.record10_result(case)
                path, r = next((p, r) for p, r in receipts if r["context"]["kind"] == "ThiserrorStaticFeatureProbe")
                self.assertEqual(r["exit_code"], code)
                self.assertEqual(len(r["outputs"]), count)
                self.assertEqual(json.loads((session / "before-builder-cleanup.json").read_text())[0]["outputs"], r["outputs"])
                self.assertFalse((Path(r["context"]["out_dir"]) / "probe").exists())
                for output in r["outputs"]:
                    self.assertEqual(sha(path / output["snapshot"]), output["sha256"])
                self.assertEqual(record["blockers"], [])
        for case in ("probe0-none", "probe2", "signal", "source-post", "capture-alias", "capture-late", "dep-error", "snapshot-missing", "snapshot-tamper"):
            with self.subTest(case=case):
                session, record, receipts = self.record10_result(case, 2)
                path, r = next((p, r) for p, r in receipts if r["context"]["kind"] == "ThiserrorStaticFeatureProbe")
                self.assertEqual(r["exit_code"], -9 if case == "signal" else (2 if case == "probe2" else (0 if case == "probe0-none" else 1)))
                self.assertTrue((path / "stdout.raw").read_bytes())
                self.assertTrue((path / "stderr.raw").read_bytes())
                self.assertTrue(record["blockers"])
                marker = {"probe0-none": "MissingDeclaredOutput:", "probe2": "CompilerFailed", "signal": "CompilerFailed",
                          "source-post": "Record10PostSource:", "capture-alias": "TransientEvidence:", "capture-late": "TransientEvidence:",
                          "dep-error": "TransientEvidence:", "snapshot-missing": "Record10Evidence:", "snapshot-tamper": "Record10Evidence:"}[case]
                self.assertTrue(any(b.startswith(marker) for b in record["blockers"]), record["blockers"])
                if case == "capture-late":
                    self.assertEqual(len(r["outputs"]), 1)
                    self.assertEqual(sha(path / r["outputs"][0]["snapshot"]), r["outputs"][0]["sha256"])

    def test_record10_final_origin_generated_source_and_cfg_joins(self):
        markers = {"missing-builder": "UnresolvedBuildScriptProducer", "duplicate-builder": "UnresolvedBuildScriptProducer",
                   "alias-bytes": "UnresolvedCargoArtifact:", "missing-origin": "Record10Graph:Record10OriginJoin",
                   "duplicate-origin": "DuplicateBuildScriptOutDir:", "origin-package": "Record10Graph:Record10OriginJoin",
                   "origin-outdir": "Record10Graph:Record10ChildJoin", "missing-consumer": "Record10Graph:Record10ConsumerJoin",
                   "duplicate-consumer": "Record10Graph:Record10ConsumerJoin", "cfg-mismatch": "Record10Graph:Record10CfgJoin",
                   "private-bytes": "Record10Graph:Record10GeneratedJoin", "generated-dep-missing": "Record10Graph:Record10GeneratedJoin",
                   "zero-child": "Record10Graph:Record10ChildJoin", "unannotated": "Record10Evidence:Record10Invocation",
                   "duplicate-child": "Record10Graph:Record10ChildJoin",
                   "builder-raw": "Record10Graph:Record10BuilderEvidence",
                   "builder-cwd": "Record10Graph:Record10BuilderEvidence",
                   "builder-request-cwd": "Record10Graph:Record10BuilderEvidence",
                   "consumer-cwd": "Record10Graph:Record10Package",
                   "consumer-initial-cwd": "Record10Graph:Record10ConsumerJoin",
                   "version-fail": "Record10Graph:Record10ChildJoin"}
        for case, marker in markers.items():
            with self.subTest(case=case):
                session, record, receipts = self.record10_result(case, 2)
                self.assertTrue(any(b.startswith(marker) for b in record["blockers"]), record["blockers"])
                if case in ("consumer-cwd", "builder-cwd"):
                    leaf = "src/lib.rs" if case == "consumer-cwd" else "build.rs"
                    path, receipt = next((p, r) for p, r in receipts if r.get("source") == str(session / "vendor/thiserror" / leaf))
                    request = json.loads((path / "request.json").read_text())
                    self.assertEqual(receipt["cwd"], str(session / "application"))
                    self.assertEqual(os.fsdecode(bytes.fromhex(request["cwd_hex"])), receipt["cwd"])
                    self.assertEqual(receipt["exit_code"], 0)
                    hit = json.loads((session / "compiler-entry" / (("thiserror" if case == "consumer-cwd" else "build_script_build") + "-thiserror.json")).read_text())
                    self.assertEqual(hit["cwd"], receipt["cwd"])
                    self.assertEqual([os.fsencode(a).hex() for a in hit["argv"]], request["argv_hex"])
                    self.assertIn(os.fsencode(str(session / "vendor/thiserror" / leaf)).hex(), request["argv_hex"])
        for case in ("probe0-both", "probe1-none"):
            session, record, receipts = self.record10_result(case)
            self.assertEqual(len(record["nested_origins"]), 2)
            for path, receipt in receipts:
                if receipt.get("source") in {str(session / "vendor/thiserror" / f) for f in ("src/lib.rs", "build.rs")}:
                    request = json.loads((path / "request.json").read_text())
                    initial = json.loads((path / "invocation.json").read_text())
                    self.assertEqual(receipt["cwd"], str(session / "vendor/thiserror"))
                    self.assertEqual(initial["cwd"], receipt["cwd"])
                    self.assertEqual(os.fsdecode(bytes.fromhex(request["cwd_hex"])), receipt["cwd"])
            for association in record["build_script_associations"]:
                filename = "version.expr" if association["package_id"].endswith("#rustversion@1.0.22") else "private.rs"
                generated = Path(association["out_dir"]) / filename
                self.assertEqual(association["generated_files"][filename], sha(generated))
                matched = [c for c in record["consumed_sources"] if c["path"] == str(generated)]
                self.assertEqual(len(matched), 1)
                self.assertEqual(matched[0]["owner"]["generated_by"], association["producer_invocation"])

    def test_record10_observation_never_creates_output_or_native_authority(self):
        markers = {"transient-artifact": "TransientCargoArtifact:", "transient-extern": "TransientExtern:",
                   "transient-consumed": "TransientConsumedSource:", "transient-ordinary": "TransientOrdinaryOutput:",
                   "version-owned-file": "Record10Evidence:Record10VersionEvidence", "probe2": "CompilerFailed",
                   "cargo-failure": "CargoDidNotFinishSuccessfully", "unannotated": "Record10Evidence:Record10Invocation"}
        for case, marker in markers.items():
            with self.subTest(case=case):
                _, record, _ = self.record10_result(case, 2)
                self.assertTrue(record["blockers"])
                self.assertTrue(any(b.startswith(marker) for b in record["blockers"]), record["blockers"])
                self.assertEqual(record["native_link_declarations"], [])
                self.assertNotIn("native_record_sha256", record)
                self.assertNotIn("qualified", record)
        cases = [("features", reuse, leaf) for reuse in ("artifact", "extern", "consumed", "ordinary") for leaf in ("dep", "meta")]
        cases += [(denial, "ordinary", leaf) for denial in ("manifest", "template", "noreceipt") for leaf in ("dep", "meta")]
        for denial, reuse, leaf in cases:
            with self.subTest(denial=denial, reuse=reuse, leaf=leaf):
                session, record, receipts = self.record10_result("namespace-" + denial + "-" + reuse + "-" + leaf, 2)
                marker = {"features": "Record10Evidence:Record10Features", "manifest": "Record10Evidence:Record10Package",
                          "template": "Record10Evidence:Record10Template", "noreceipt": "IncompleteInvocation:"}[denial]
                self.assertTrue(any(b.startswith(marker) for b in record["blockers"]), record["blockers"])
                out = session / "target/x86_64-apple-darwin/debug/build/thiserror-0123456789abcdef/out"
                names = {str(out / "probe" / f) for f in ("thiserror.d", "libthiserror.rmeta")}
                forbidden = str(out / "probe" / ("thiserror.d" if leaf == "dep" else "libthiserror.rmeta"))
                exclude = {"artifact": "TransientCargoArtifact:", "extern": "TransientExtern:",
                           "consumed": "TransientConsumedSource:", "ordinary": "TransientOrdinaryOutput:"}[reuse]
                self.assertIn(exclude + forbidden, record["blockers"])
                self.assertTrue(all(c["path"] not in names for c in record["consumed_sources"]))
                self.assertTrue(all(e["path"] not in names or e["producers"] == [] for e in record["extern_edges"]))
                if reuse == "extern":
                    self.assertTrue(any(e["path"] == forbidden and e["producers"] == [] for e in record["extern_edges"]))
                self.assertTrue(all(not names.intersection(a["files"]) for a in record["selected_library"]))
                self.assertTrue(all(not any(str(Path(a["out_dir"]) / f) in names for f in a["generated_files"]) for a in record["build_script_associations"]))
                if denial == "noreceipt":
                    self.assertFalse((session / "compiler-entry/feature-probe-thiserror.json").exists())
                    self.assertTrue(any(not (p.parent / "receipt.json").exists() for p in (session / "invocations").glob("*/request.json")))
                else:
                    child_path, child = next((p, r) for p, r in receipts if r["source"] == str(session / "vendor/thiserror/build/probe.rs"))
                    request = json.loads((child_path / "request.json").read_text())
                    self.assertEqual(child["kind"], "Compile")
                    self.assertEqual(child["context"], {"kind": "DirectCargoCompile"})
                    self.assertEqual(request["environment_hex"], child["environment_hex"])
                self.assertEqual(record["native_link_declarations"], [])
                self.assertNotIn("native_record_sha256", record)
        session, record, receipts = self.record10_result("probe1-none")
        paths = {o["path"] for _, r in receipts if r["kind"] == "TransientProbe" for o in r["declared_outputs"]}
        self.assertEqual(len(paths), 2)
        self.assertTrue(all(not any(c["path"] == p for c in record["consumed_sources"]) for p in paths))
        self.assertEqual(record["state"], "RecordingOnly")
        self.assertEqual(record["review_gate"], "IndependentPolicyReviewRequired")

    def prepare_psm11(self, case="normal"):
        inv = self.prepare(); vendor = self.root / "vendor-origin"
        specs = (("psm", "0.1.30"), ("cc", "1.2.59"), ("ar_archive_writer", "0.5.1"))
        for name, version in specs:
            write(vendor / name / "Cargo.toml", '[package]\nname="' + name + '"\nversion="' + version + '"\n')
            write(vendor / name / "src/lib.rs", "// TEST_CODE synthetic " + name + "\n")
            write(vendor / name / ".cargo-checksum.json", '{"files":{},"package":"TEST_CODE"}')
            inv["packages"].append({"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
                                    "tree": "vendor", "manifest": name + "/Cargo.toml"})
        for source in ("build.rs", "src/arch/x86_64.s", "src/arch/psm.h", "src/arch/gnu_stack_note.s"):
            write(vendor / "psm" / source, "// TEST_CODE separately inventoried " + source + "\n")
        inv["vendor"] = snapshot(vendor, ["dep", *[n for n, _ in specs]])
        rustc = write(self.root / "fake-rustc", "#!" + PYTHON + " -I\n" + PSM11_RUSTC)
        cargo = write(self.root / "fake-cargo", "#!" + PYTHON + " -I\n" + PSM11_CARGO.replace("__CASE__", repr(case)).replace("__TEMPLATE__", repr(PSM11_ARGS)))
        rustc.chmod(0o700); cargo.chmod(0o700)
        inv["rustc"] = {"path": str(rustc), "sha256": sha(rustc)}; inv["cargo"] = {"path": str(cargo), "sha256": sha(cargo)}
        inv["generators"]["PROTOC"] = dict(inv["rustc"])
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly", "profile": owner.PROFILE, "inventory": inv}))
        return inv

    def psm11_result(self, case="normal", status=0):
        self.prepare_psm11(case); result = self.invoke("record")
        self.assertEqual(result.returncode, status, result.stderr.decode(errors="replace"))
        record = self.record_result(result); session = Path(json.loads(result.stdout)["record_path"]).parent
        receipts = [(p.parent, json.loads(p.read_text())) for p in (session / "invocations").glob("*/receipt.json")]
        return session, record, receipts

    def psm11_before_entry_refusal(self, case, marker):
        session, record, receipts = self.psm11_result(case, 2)
        self.assertFalse((session / "compiler-entry/compile-psm").exists())
        self.assertEqual(record["native_link_declarations"], [])
        diagnostics = [json.loads(line) for line in (session / "cargo.stderr.raw").read_text().splitlines()]
        self.assertTrue(any(d.get("reason") == "Refused" and d.get("detail") == marker for d in diagnostics), diagnostics)
        requests = [p for p in (session / "invocations").glob("*/request.json") if not (p.parent / "receipt.json").exists()]
        self.assertEqual(len(requests), 1)
        self.assertEqual(json.loads(requests[0].read_text())["argv_hex"], json.loads((session / "psm11-attempt.json").read_text())["argv_hex"])
        self.assertIn("IncompleteInvocation:" + requests[0].parent.name, record["blockers"])
        return session, record

    def test_record11_psm_connected_graph_and_raw_indices(self):
        session, record, receipts = self.psm11_result()
        self.assertEqual(record["blockers"], []); self.assertEqual(len(record["selected_library"]), 1)
        path, r = next((p, r) for p, r in receipts if r.get("context", {}).get("psm_static_declaration"))
        declaration, = record["native_link_declarations"]
        raw = [os.fsdecode(bytes.fromhex(a)) for a in r["argv_hex"]]
        self.assertEqual(raw, [raw[0]] + [v.format(source=session / "vendor/psm/src/lib.rs", deps=session / "target" / owner.TARGET / "debug/deps", host=session / "target/debug/deps", out=r["context"]["out_dir"]) for v in PSM11_ARGS])
        self.assertEqual([raw[i] for i in declaration["raw_argument_indices"]], ["-L", "native=" + r["context"]["out_dir"], "-l", "static=psm_s"])
        self.assertEqual(declaration["consumer_invocation"], r["invocation_id"] if "invocation_id" in r else path.name)
        self.assertEqual(declaration["artifact_selection"], "not_observed"); self.assertEqual(declaration["native_child_provenance"], "not_observed")
        self.assertEqual(declaration["native_producer_qualification"], "not_issued")
        builder = declaration["producer_invocation"]
        self.assertEqual({e["name"] for e in record["extern_edges"] if e["consumer"] == builder}, {"cc", "ar_archive_writer"})
        for phase in ("pre", "post"):
            row = r["psm_archive_" + phase]; self.assertEqual(sha(path / row["snapshot"]), row["sha256"])
        unrelated = [str(self.root / "fake-rustc"), "--crate-name", "other", "--edition=2021", str(session / "application/src/lib.rs"), "--crate-type", "lib", "--emit=dep-info,metadata,link", "--out-dir", str(session / "target" / owner.TARGET / "debug/deps")]
        self.assertIsNone(owner.psm_static_context(unrelated, {}, session / "vendor/psm", session, {}))

    def test_record11_psm_identity_and_missing_native_fail_before_entry(self):
        cases = {"source": "PsmStaticTemplate", "package": "PsmSourceContext", "version": "PsmSourceContext",
                 "cwd": "PsmSourceContext", "outdir": "PsmSourceContext", "source_hash": "PsmSourceContext",
                 "feature": "PsmEnvironmentContext", "environment": "PsmEnvironmentContext", "wrapper": "PsmEnvironmentContext",
                 "no_native": "PsmStaticTemplate", "no_suffix": "PsmStaticTemplate", "host": "PsmStaticTemplate", "platform": "PsmStaticTemplate",
                 "source_alias_no_native": "PsmSourceContext", "manifest_alias_no_native": "PsmSourceContext"}
        for case, marker in cases.items():
            with self.subTest(case=case): self.psm11_before_entry_refusal(case, marker)

    def test_record11_psm_closed_native_cfg_and_check_cfg_tokens(self):
        for case in ("inline", "reorder", "extra_native", "extra_cfg", "check_cfg", "link_arg", "extern", "extern_native", "test"):
            with self.subTest(case=case): self.psm11_before_entry_refusal(case, "PsmStaticTemplate")

    def test_record11_psm_request_evidence_and_exact_graph_joins(self):
        cases = {"request_only": "IncompleteInvocation:", "missing_annotation": "PsmGraph:PsmInvocationEvidence",
                 "request_cwd": "PsmEvidence:PsmSourceContext", "raw_builder": "PsmEvidence:PsmSourceContext",
                 "missing_builder": "PsmGraph:PsmOrigin", "duplicate_builder": "PsmGraph:PsmOrigin",
                 "missing_event": "PsmGraph:PsmOrigin", "duplicate_event": "PsmGraph:PsmOrigin", "event_outdir": "PsmGraph:PsmOrigin",
                 "event_cfg": "PsmGraph:PsmDeclaration", "event_env": "PsmGraph:PsmDeclaration", "builder_features": "PsmGraph:PsmBuilder",
                 "missing_cc": "PsmGraph:PsmBuilderExtern", "duplicate_cc": "PsmGraph:PsmBuilderExtern",
                 "missing_ar_archive_writer": "PsmGraph:PsmBuilderExtern", "duplicate_ar_archive_writer": "PsmGraph:PsmBuilderExtern",
                 "missing_consumer": "PsmGraph:PsmConsumer", "duplicate_consumer": "PsmGraph:PsmConsumer",
                 "consumer_features": "PsmGraph:PsmConsumer", "consumer_role": "PsmGraph:PsmConsumer",
                 "target_helper_cc": "PsmGraph:PsmBuilderExtern", "target_helper_ar_archive_writer": "PsmGraph:PsmBuilderExtern",
                 "helper_cwd_metadata": "PsmGraph:PsmBuilderExtern"}
        for case, marker in cases.items():
            with self.subTest(case=case):
                session, record, _ = self.psm11_result(case, 2)
                self.assertTrue((session / "compiler-entry/compile-psm").exists())
                self.assertTrue(any(b.startswith(marker) for b in record["blockers"]), record["blockers"])
                self.assertEqual(record["native_link_declarations"], [])

    def test_record11_psm_archive_observations_and_failed_status(self):
        for case in ("pre_missing", "pre_symlink", "pre_hardlink"):
            with self.subTest(case=case): self.psm11_before_entry_refusal(case, "PsmArchiveEvidence")
        cases = {"during_change": "PsmArchiveChanged", "post_missing": "PsmArchiveEvidence", "post_symlink": "PsmArchiveEvidence",
                 "post_hardlink": "PsmArchiveEvidence", "snapshot_missing": "PsmGraph:PsmArchiveBinding",
                 "snapshot_changed": "PsmGraph:PsmArchiveBinding", "final_archive": "PsmGraph:PsmArchiveBinding",
                 "source_post": "PsmPostSource:PsmSourceContext", "compiler_fail": "CompilerFailed"}
        for case, marker in cases.items():
            with self.subTest(case=case):
                session, record, receipts = self.psm11_result(case, 2)
                self.assertTrue((session / "compiler-entry/compile-psm").exists())
                self.assertIn(marker, record["blockers"]); self.assertEqual(record["native_link_declarations"], [])
                if case == "compiler_fail":
                    _, r = next((p, r) for p, r in receipts if r.get("context", {}).get("psm_static_declaration"))
                    self.assertEqual(r["exit_code"], 7); self.assertIn("psm_archive_post", r)
        from unittest.mock import patch
        call = self.root / "TEST_CODE_partial_read"; call.mkdir(); out = call / "out"; out.mkdir()
        (out / "libpsm_s.a").write_bytes(b"TEST_CODE_NONEMPTY_ARCHIVE")
        with patch.object(owner.shutil, "copyfileobj", side_effect=lambda source, dest, size: dest.write(source.read(1))):
            with self.assertRaisesRegex(owner.Refusal, "PsmArchiveEvidence"):
                owner.psm_archive_observation(call, {"out_dir": str(out)}, "pre")

    def test_record11_psm_archive_cannot_become_ordinary_ownership(self):
        cases = {"archive_output": "PsmArchiveOwnership:Output", "snapshot_output": "PsmArchiveOwnership:Output",
                 "archive_extern": "PsmArchiveOwnership:Extern", "archive_consumed": "PsmArchiveOwnership:ConsumedSource",
                 "archive_selected": "PsmArchiveOwnership:CargoArtifact", "archive_copy": "PsmArchiveOwnership:CargoArtifact",
                 "archive_retained_copy": "PsmArchiveOwnership:CargoArtifact", "archive_current_output": "PsmArchiveOwnership:Output",
                 "archive_retained_extern": "PsmArchiveOwnership:Extern", "archive_retained_consumed": "PsmArchiveOwnership:ConsumedSource"}
        for case, marker in cases.items():
            with self.subTest(case=case):
                _, record, _ = self.psm11_result(case, 2)
                self.assertIn(marker, record["blockers"]); self.assertEqual(record["native_link_declarations"], [])
                self.assertFalse(any(c["path"].endswith("libpsm_s.a") for c in record["consumed_sources"]))



    def prepare_tools12(self, family, case="normal"):
        inv = self.prepare(); vendor = self.root / "vendor-origin"
        specs = {"zstd": (("zstd-sys", "2.0.16+zstd.1.5.7", ("Cargo.toml", "build.rs", "src/lib.rs", "src/bindings_zstd.rs", "src/bindings_zdict.rs")),
                           ("cc", "1.2.59", ("Cargo.toml", "src/lib.rs")), ("pkg-config", "0.3.32", ("Cargo.toml", "src/lib.rs"))),
                 "anyhow": (("anyhow", "1.0.102", ("Cargo.toml", "build.rs", "src/backtrace.rs", "src/chain.rs", "src/context.rs", "src/ensure.rs", "src/error.rs", "src/fmt.rs", "src/kind.rs", "src/lib.rs", "src/macros.rs", "src/nightly.rs", "src/ptr.rs", "src/wrapper.rs")),),
                 "serde": (("serde_core", "1.0.228", ("Cargo.toml", "build.rs", "src/crate_root.rs", "src/de/ignored_any.rs", "src/de/impls.rs", "src/de/mod.rs", "src/de/value.rs", "src/format.rs", "src/lib.rs", "src/macros.rs", "src/private/content.rs", "src/private/doc.rs", "src/private/mod.rs", "src/private/seed.rs", "src/private/size_hint.rs", "src/private/string.rs", "src/ser/fmt.rs", "src/ser/impls.rs", "src/ser/impossible.rs", "src/ser/mod.rs", "src/std_error.rs")),)}[family]
        for name, version, sources in specs:
            for leaf in sources:
                write(vendor / name / leaf, '[package]\nname="' + name + '"\nversion="' + version + '"\n' if leaf == "Cargo.toml" else "// TEST_CODE separately inventoried " + name + "/" + leaf + "\n")
            write(vendor / name / ".cargo-checksum.json", '{"files":{},"package":"TEST_CODE"}')
            inv["packages"].append({"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
                                    "tree": "vendor", "manifest": name + "/Cargo.toml"})
        inv["vendor"] = snapshot(vendor, ["dep", *[n for n, _, _ in specs]])
        write(self.root / "sysroot/lib/rustlib/x86_64-apple-darwin/lib/test.bin", "TEST_CODE_HOST_SYSROOT")
        inv["sysroot"] = snapshot(self.root / "sysroot", ["lib"])
        rustc = write(self.root / "fake-rustc", "#!" + PYTHON + " -I\n" + TOOLS12_RUSTC)
        cargo_text = TOOLS12_CARGO.replace("__CASE__", repr(case)).replace("__FAMILY__", repr(family)).replace("__ZSTD__", repr(ZSTD12_ARGS)).replace("__SCHECK__", repr(SERDE12_CHECK)).replace("__CHECKS__", repr(SERDE12_CHECKS)).replace("__PRIVATE__", repr(TOOLS12_PRIVATE))
        cargo = write(self.root / "fake-cargo", "#!" + PYTHON + " -I\n" + cargo_text)
        rustc.chmod(0o700); cargo.chmod(0o700)
        inv["rustc"] = {"path": str(rustc), "sha256": sha(rustc)}; inv["cargo"] = {"path": str(cargo), "sha256": sha(cargo)}
        inv["generators"]["PROTOC"] = dict(inv["rustc"])
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly", "profile": owner.PROFILE, "inventory": inv}))
        return inv

    def tools12_result(self, family, case="normal", status=0):
        self.prepare_tools12(family, case); result = self.invoke("record")
        self.assertEqual(result.returncode, status, result.stderr.decode(errors="replace"))
        record = self.record_result(result); session = Path(json.loads(result.stdout)["record_path"]).parent
        receipts = [(p.parent, json.loads(p.read_text())) for p in (session / "invocations").glob("*/receipt.json")]
        return session, record, receipts

    def tools12_admission_refusal(self, family, case, marker):
        session, record, receipts = self.tools12_result(family, case, 2)
        name = "compile-zstd_sys" if family == "zstd" else "probe-anyhow"
        self.assertFalse((session / "compiler-entry" / name).exists())
        self.assertTrue(any(r["source"] == str(session / "vendor" / ("zstd-sys" if family == "zstd" else "anyhow") / "build.rs") for _, r in receipts))
        diagnostics = [json.loads(line) for line in (session / "cargo.stderr.raw").read_text().splitlines()]
        self.assertTrue(any(d.get("reason") == "Refused" and d.get("detail") == marker for d in diagnostics), diagnostics)
        requests = [p for p in (session / "invocations").glob("*/request.json") if not (p.parent / "receipt.json").exists()]
        self.assertEqual(len(requests), 1)
        self.assertEqual(json.loads(requests[0].read_text())["argv_hex"], json.loads((session / "tools12-attempt.json").read_text())["argv_hex"])
        self.assertIn("IncompleteInvocation:" + requests[0].parent.name, record["blockers"])
        self.assertEqual(record["native_link_declarations"], [])

    def test_record12_zstd_connected_native_declaration_and_archive_graph(self):
        session, record, receipts = self.tools12_result("zstd")
        self.assertEqual(record["blockers"], []); self.assertEqual(len(record["selected_library"]), 1)
        path, r = next((p, r) for p, r in receipts if r["context"].get("zstd_static_declaration"))
        declaration, = record["native_link_declarations"]
        raw = [os.fsdecode(bytes.fromhex(a)) for a in r["argv_hex"]]
        self.assertEqual(len(raw), 45)
        self.assertEqual(raw[1:], [v.format(source=session / "vendor/zstd-sys/src/lib.rs", deps=session / "target" / owner.TARGET / "debug/deps", host=session / "target/debug/deps", out=r["context"]["out_dir"]) for v in ZSTD12_ARGS])
        self.assertEqual(declaration["raw_argument_indices"], [41, 42, 43, 44])
        self.assertEqual([raw[i] for i in declaration["raw_argument_indices"]], ["-L", "native=" + r["context"]["out_dir"], "-l", "static=zstd"])
        self.assertEqual(declaration["state"], "RecordingOnly"); self.assertEqual(declaration["consumer_invocation"], path.name)
        self.assertEqual(declaration["artifact_selection"], "not_observed"); self.assertEqual(declaration["native_child_provenance"], "not_observed")
        self.assertEqual(declaration["native_producer_qualification"], "not_issued")
        builder = declaration["producer_invocation"]
        self.assertEqual({e["name"] for e in record["extern_edges"] if e["consumer"] == builder}, {"cc", "pkg_config"})
        by_id = {p.name: row for p, row in receipts}
        for edge in [e for e in record["extern_edges"] if e["consumer"] == builder]:
            producer, = edge["producers"]; self.assertEqual(by_id[producer]["role"], "Host"); self.assertEqual(by_id[producer]["exit_code"], 0)
        for phase in ("pre", "post"):
            row = r["zstd_archive_" + phase]
            self.assertEqual(sha(path / row["snapshot"]), row["sha256"]); self.assertEqual(sha(Path(row["path"])), row["sha256"])
        self.assertFalse(any(c["path"].endswith("libzstd.a") for c in record["consumed_sources"]))
        self.assertEqual(record["selected_library"][0]["package_id"], "TEST_CODE_app")

    def test_record12_zstd_source_raw_status_snapshots_and_negative_namespace(self):
        admission = {"source": "ZstdStaticTemplate", "package": "ZstdSourceContext", "version": "ZstdSourceContext", "cwd": "ZstdSourceContext", "outdir": "ZstdSourceContext", "source_hash": "ZstdSourceContext",
            "feature": "ZstdEnvironmentContext", "bootstrap": "ZstdEnvironmentContext", "stage": "ZstdEnvironmentContext", "wrapper": "ZstdEnvironmentContext", "inline": "ZstdStaticTemplate", "raw_space": "ZstdStaticTemplate", "raw_feature": "ZstdStaticTemplate", "source_alias": "ZstdStaticTemplate", "manifest_alias": "ZstdSourceContext", "reorder": "ZstdStaticTemplate", "extra_native": "ZstdStaticTemplate", "no_native": "ZstdStaticTemplate", "extern": "ZstdStaticTemplate", "link_arg": "ZstdStaticTemplate", "host": "ZstdStaticTemplate", "platform": "ZstdStaticTemplate", "pre_missing": "ZstdArchiveEvidence", "pre_symlink": "ZstdArchiveEvidence", "pre_hardlink": "ZstdArchiveEvidence"}
        for case, marker in admission.items():
            with self.subTest(admission=case): self.tools12_admission_refusal("zstd", case, marker)
        after = {"during_change": "ZstdArchiveChanged", "post_missing": "ZstdArchiveEvidence", "post_symlink": "ZstdArchiveEvidence", "post_hardlink": "ZstdArchiveEvidence", "source_post": "ZstdPostSource:ZstdSourceContext", "compiler_fail": "CompilerFailed",
            "snapshot_missing": "ZstdGraph:ZstdArchiveBinding", "snapshot_changed": "ZstdGraph:ZstdArchiveBinding", "final_archive": "ZstdGraph:ZstdArchiveBinding", "request_only": "IncompleteInvocation:", "missing_annotation": "ZstdGraph:ZstdInvocationEvidence", "request_cwd": "ZstdGraph:ZstdSourceContext", "raw_builder": "ZstdGraph:ZstdSourceContext", "builder_features": "ZstdGraph:ZstdBuilder", "missing_builder": "ZstdGraph:ZstdOrigin", "duplicate_builder": "ZstdGraph:ZstdOrigin", "missing_event": "ZstdGraph:ZstdOrigin", "duplicate_event": "ZstdGraph:ZstdOrigin", "event_outdir": "ZstdGraph:ZstdOrigin", "event_cfg": "ZstdGraph:ZstdDeclaration", "event_env": "ZstdGraph:ZstdDeclaration", "missing_cc": "ZstdGraph:ZstdBuilderExtern", "duplicate_cc": "ZstdGraph:ZstdBuilderExtern", "missing_pkg_config": "ZstdGraph:ZstdBuilderExtern", "target_helper_cc": "ZstdGraph:ZstdBuilderExtern", "target_helper_pkg_config": "ZstdGraph:ZstdBuilderExtern", "missing_consumer": "ZstdGraph:ZstdConsumer", "duplicate_consumer": "ZstdGraph:ZstdConsumer", "consumer_features": "ZstdGraph:ZstdConsumer"}
        for case, marker in after.items():
            with self.subTest(after=case):
                session, record, receipts = self.tools12_result("zstd", case, 2)
                self.assertTrue((session / "compiler-entry/compile-zstd_sys").exists())
                self.assertTrue(any(b.startswith(marker) for b in record["blockers"]), record["blockers"])
                self.assertEqual(record["native_link_declarations"], [])
                if case == "compiler_fail":
                    _, r = next((p, r) for p, r in receipts if r["context"].get("zstd_static_declaration"))
                    self.assertEqual(r["exit_code"], 7); self.assertIn("zstd_archive_post", r)
        for epoch in ("current", "retained"):
            for reuse, marker in (("output", "Output"), ("artifact", "CargoArtifact"), ("extern", "Extern"), ("consumed", "ConsumedSource"), ("selected", "CargoArtifact"), ("snapshot", "Output")):
                with self.subTest(epoch=epoch, reuse=reuse):
                    session, record, _ = self.tools12_result("zstd", "namespace_" + epoch + "_" + reuse, 2)
                    self.assertTrue((session / "compiler-entry/compile-zstd_sys").exists())
                    self.assertIn("ZstdArchiveOwnership:" + marker, record["blockers"]); self.assertEqual(record["native_link_declarations"], [])
        from unittest.mock import patch
        call = self.root / "TEST_CODE_zstd_partial"; call.mkdir(); (call / "libzstd.a").write_bytes(b"TEST_CODE_NONEMPTY_ARCHIVE")
        with patch.object(owner.shutil, "copyfileobj", side_effect=lambda source, dest, size: dest.write(source.read(1))):
            with self.assertRaisesRegex(owner.Refusal, "ZstdArchiveEvidence"):
                owner.static_archive_observation(call, {"out_dir": str(call)}, "pre", "zstd", "libzstd.a", "ZstdArchiveEvidence")

    def test_record12_anyhow_actual_compiler_status_and_cleanup_capture(self):
        for case, code, outputs in (("normal", 0, 2), ("probe1-none", 1, 0), ("probe1-partial", 1, 1), ("probe1-both", 1, 2)):
            with self.subTest(case=case):
                session, record, receipts = self.tools12_result("anyhow", case)
                self.assertEqual(record["blockers"], []); self.assertEqual(len(record["selected_library"]), 1)
                self.assertTrue((session / "compiler-entry/compile-anyhow").exists())
                _, consumer = next((p, r) for p, r in receipts if r["source"] == str(session / "vendor/anyhow/src/lib.rs"))
                self.assertEqual((consumer["kind"], consumer["role"], consumer["context"], consumer["exit_code"]), ("Compile", "Host", {"kind": "DirectCargoCompile"}, 0))
                self.assertEqual(consumer["parsed"]["options"]["--check-cfg"], ["cfg(" + c + ")" for c in ("anyhow_build_probe", "anyhow_nightly_testing", "anyhow_no_clippy_format_args", "anyhow_no_core_error", "error_generic_member_access")])
                self.assertNotIn("anyhow_build_probe", consumer["parsed"]["options"].get("--cfg", []))
                consumer_env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in consumer["environment_hex"].items()}
                self.assertFalse(any(k.startswith("CARGO_FEATURE_") for k in consumer_env))
                self.assertEqual(consumer_env["DYLD_FALLBACK_LIBRARY_PATH"].split(":")[0], str(session / "target/debug/deps"))
                self.assertEqual(len(consumer_env["DYLD_FALLBACK_LIBRARY_PATH"].split(":")), 2)
                path, r = next((p, r) for p, r in receipts if r["context"]["kind"] == "AnyhowStaticFeatureProbe")
                self.assertEqual(r["exit_code"], code); self.assertEqual(r["probe_outcome"], "Supported" if code == 0 else "Unsupported")
                self.assertEqual(r["role"], "Target"); self.assertEqual(r["kind"], "TransientProbe"); self.assertEqual(len(r["outputs"]), outputs)
                self.assertFalse((Path(r["context"]["out_dir"]) / "probe").exists())
                raw = [os.fsdecode(bytes.fromhex(a)) for a in r["argv_hex"]]
                self.assertEqual(raw[1:], ["--cfg=anyhow_build_probe", "--edition=2018", "--crate-name=anyhow", "--crate-type=lib", "--cap-lints=allow", "--emit=dep-info,metadata", "--out-dir", str(Path(r["context"]["out_dir"]) / "probe"), "src/nightly.rs", "--target", owner.TARGET])
                for output in r["outputs"]:self.assertEqual(sha(path / output["snapshot"]), output["sha256"])
                association, = [a for a in record["build_script_associations"] if a["package_id"].endswith("#anyhow@1.0.102")]
                self.assertEqual(association["generated_files"], {}); self.assertEqual(association["cargo_event"]["cfgs"], ["error_generic_member_access"] if code == 0 else [])
                origin, = [o for o in record["nested_origins"] if o["invocation_id"] == path.name]
                self.assertEqual(origin["producer_invocation"], association["producer_invocation"])
                self.assertFalse(any("/probe/" in c["path"] for c in record["consumed_sources"]))
                self.assertEqual(record["native_link_declarations"], [])
        session, record, receipts = self.tools12_result("anyhow", "probe7", 2)
        _, r = next((p, r) for p, r in receipts if r["context"]["kind"] == "AnyhowStaticFeatureProbe")
        self.assertEqual(r["exit_code"], 7); self.assertEqual(r["probe_outcome"], "CompilerFailure"); self.assertIn("CompilerFailed", record["blockers"])
        for case, marker in (("capture_missing", "MissingDeclaredOutput:"), ("capture_alias", "TransientEvidence:TransientOutputAlias"), ("capture_hardlink", "TransientEvidence:TransientOutputAlias"), ("capture_dep", "TransientEvidence:UnsupportedDepComment"), ("source_post", "AnyhowPostSource:AnyhowSourceContext")):
            with self.subTest(evidence=case):
                session, record, receipts = self.tools12_result("anyhow", case, 2)
                _, r = next((p, r) for p, r in receipts if r["context"]["kind"] == "AnyhowStaticFeatureProbe")
                self.assertEqual(r["exit_code"], 0); self.assertTrue(any(b.startswith(marker) for b in record["blockers"]), record["blockers"])

    def test_record12_anyhow_source_raw_origin_and_transient_namespace_refuse(self):
        admission = {"source": "AnyhowTemplate", "package": "AnyhowSourceContext", "version": "AnyhowSourceContext", "cwd": "AnyhowSourceContext", "source_hash": "AnyhowSourceContext", "outdir": "AnyhowOutDir", "feature": "AnyhowFeatures", "bootstrap": "AnyhowEnvironment", "stage": "AnyhowEnvironment", "wrapper": "AnyhowEnvironment", "inline": "AnyhowTemplate", "raw_space": "AnyhowTemplate", "host": "AnyhowTemplate", "platform": "AnyhowTemplate", "retry": "AnyhowTemplate", "source_alias": "AnyhowTemplate", "manifest_alias": "AnyhowSourceContext", "host_env": "AnyhowEnvironment", "encoded_flags": "AnyhowEnvironment", "missing_feature": "AnyhowFeatures", "loader": "CompilerEnvironmentInjection", "probe_check_cfg": "AnyhowTemplate", "probe_check_cfg_source": "AnyhowTemplate"}
        for case, marker in admission.items():
            with self.subTest(admission=case):self.tools12_admission_refusal("anyhow", case, marker)
        after = {"request_only": "IncompleteInvocation:", "missing_annotation": "AnyhowGraph:CompilerEnvironmentInjection", "request_cwd": "AnyhowGraph:AnyhowSourceContext", "raw_builder": "AnyhowGraph:AnyhowBuilderJoin", "builder_features": "AnyhowGraph:AnyhowBuilderJoin", "missing_builder": "AnyhowGraph:AnyhowOriginJoin", "duplicate_builder": "AnyhowGraph:AnyhowOriginJoin", "missing_event": "AnyhowGraph:AnyhowOriginJoin", "duplicate_event": "AnyhowGraph:AnyhowOriginJoin", "event_outdir": "AnyhowGraph:AnyhowOriginJoin", "event_cfg": "AnyhowGraph:AnyhowCfgJoin", "event_env": "AnyhowGraph:AnyhowCfgJoin", "missing_consumer": "AnyhowGraph:AnyhowConsumerJoin", "duplicate_consumer": "AnyhowGraph:AnyhowConsumerJoin", "consumer_features": "AnyhowGraph:AnyhowConsumerJoin", "snapshot_missing": "AnyhowGraph:AnyhowInvocationEvidence", "snapshot_changed": "AnyhowGraph:AnyhowSnapshot"}
        for case, marker in after.items():
            with self.subTest(after=case):
                session, record, receipts = self.tools12_result("anyhow", case, 2)
                self.assertTrue((session / "compiler-entry/probe-anyhow").exists())
                self.assertTrue(any(b.startswith(marker) for b in record["blockers"]), record["blockers"])
                if case == "missing_annotation":
                    self.assertIn("Tools12Evidence:CompilerEnvironmentInjection", record["blockers"])
                    self.assertEqual(record["nested_origins"], [])
                    self.assertFalse(any("/probe/" in c["path"] for c in record["consumed_sources"]))
                    self.assertEqual(record["native_link_declarations"], [])
                if case in ("snapshot_missing", "snapshot_changed"):
                    call, r = next((p, r) for p, r in receipts if r["context"]["kind"] == "AnyhowStaticFeatureProbe")
                    self.assertEqual(r["exit_code"], 0)
                    if case == "snapshot_missing":
                        missing = call / "probe-output-0.raw"
                        self.assertFalse(missing.exists())
                        self.assertIn("Tools12Evidence:" + str(FileNotFoundError(2, os.strerror(2), str(missing))), record["blockers"])
                    else:
                        self.assertIn("Tools12Evidence:AnyhowSnapshot", record["blockers"])
                    self.assertFalse(any("/probe/" in c["path"] for c in record["consumed_sources"]))
                    self.assertEqual(record["native_link_declarations"], [])
        for epoch in ("current", "retained"):
            for reuse, marker in (("output", "Output"), ("artifact", "CargoArtifact"), ("extern", "Extern"), ("consumed", "ConsumedSource"), ("selected", "CargoArtifact"), ("snapshot", "Output")):
                with self.subTest(epoch=epoch, reuse=reuse):
                    session, record, _ = self.tools12_result("anyhow", "namespace_" + epoch + "_" + reuse, 2)
                    self.assertTrue((session / "compiler-entry/probe-anyhow").exists()); self.assertIn("AnyhowTransientOwnership:" + marker, record["blockers"])
                    self.assertFalse(any(c["path"].endswith("libpromoted.rlib") for c in record["consumed_sources"]))
                    self.assertEqual(record["native_link_declarations"], [])

    def test_record12_serde_core_two_host_builders_map_one_target_consumer(self):
        session, record, receipts = self.tools12_result("serde")
        self.assertEqual(record["blockers"], []); self.assertEqual(len(record["selected_library"]), 1)
        builders = [(p, r) for p, r in receipts if r["source"] == str(session / "vendor/serde_core/build.rs")]
        self.assertEqual(len(builders), 2)
        self.assertTrue(all(r["role"] == "Host" and r["exit_code"] == 0 and "--target" not in r["parsed"]["options"] for _, r in builders))
        consumer_path, consumer = next((p, r) for p, r in receipts if r["source"] == str(session / "vendor/serde_core/src/lib.rs"))
        self.assertEqual(consumer["role"], "Target"); self.assertEqual(consumer["parsed"]["options"]["--target"], [owner.TARGET])
        association, = [a for a in record["build_script_associations"] if a["package_id"].endswith("#serde_core@1.0.228")]
        wide_path, wide = next((p, r) for p, r in builders if 'feature="alloc"' in r["parsed"]["options"]["--cfg"])
        self.assertEqual(association["producer_invocation"], wide_path.name)
        mapping = association["recording_only_mapping"]
        self.assertEqual(mapping, {"state": "RecordingOnly", "rule": "RecordingOnlySerdeCoreConsumerFeatureMappingV1", "execution_edge": "not_observed", "consumer_invocation": consumer_path.name, "features": ["alloc", "default", "rc", "result", "std"]})
        private, = [c for c in record["consumed_sources"] if c["path"].endswith("/serde_core-8c92ebf254a84c42/out/private.rs")]
        self.assertEqual(private["owner"]["generated_by"], wide_path.name); self.assertEqual(private["owner"]["recording_only_mapping"], mapping)
        self.assertEqual(Path(private["path"]).read_bytes(), TOOLS12_PRIVATE); self.assertEqual(private["sha256"], hashlib.sha256(TOOLS12_PRIVATE).hexdigest())
        self.assertEqual(record["native_link_declarations"], []); self.assertEqual(record["nested_origins"], [])

    def test_record12_serde_core_feature_mapping_requires_all_finite_evidence(self):
        for case in ("missing_builder", "duplicate_features", "no_compatible", "private_hash_same", "builder_features", "builder_metadata", "builder_role", "builder_outdir", "raw_builder", "alias", "consumer_role", "consumer_metadata", "consumer_features", "missing_consumer", "duplicate_consumer", "event_outdir", "event_cfg", "event_env", "private_bytes", "consumer_source", "builder_source", "consumer_outdir", "compiler_identity", "builder_cwd", "package", "version", "cwd", "outdir", "feature"):
            with self.subTest(case=case):
                session, record, receipts = self.tools12_result("serde", case, 2)
                self.assertTrue((session / "compiler-entry/compile-serde_core").exists())
                self.assertIn("SerdeCoreMapping:SerdeCoreMapping", record["blockers"])
                self.assertTrue(any(r["source"] == str(session / "vendor/serde_core/build.rs") for _, r in receipts))
                self.assertFalse(any(a["package_id"].endswith("#serde_core@1.0.228") for a in record["build_script_associations"]))
                self.assertFalse(any(c["path"].endswith("private.rs") for c in record["consumed_sources"]))



    def prepare_native_e4(self, case="normal"):
        inventory = self.prepare_native_e1("normal")
        vendor = self.root / "vendor-origin"
        fixture = TOOL.parent.parent / "fixture-source"
        if not fixture.is_dir():
            fixture = TOOL.parent.parent / ".superpowers/sdd/remaining-development-20261003/native-lz4-zstd-context-source/fixture-source"
        for member, expected in owner.FOREIGN_E_ONLY_SOURCE_PINS.items():
            leaf = fixture / member
            self.assertEqual(sha(leaf), expected); self.assertFalse(leaf.is_symlink())
            destination = vendor / member
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(leaf.read_bytes())
        for name, (version, _, _, _, _) in E4_FACTS.items():
            write(vendor / name / ".cargo-checksum.json", '{"files":{},"package":"TEST_CODE"}')
            inventory["packages"].append({"id": "registry+https://github.com/rust-lang/crates.io-index#" + name + "@" + version,
                                           "tree": "vendor", "manifest": name + "/Cargo.toml"})
        inventory["vendor"] = snapshot(vendor, ["dep", "libsqlite3-sys", "cc", "diesel", "rusqlite", "ring", "psm", *E4_FACTS])
        cargo = write(self.root / "fake-cargo", "#!" + PYTHON + " -I\n" + E4_CARGO.replace("__CASE__", repr(case)).replace("__E4_FACTS__", repr(E4_FACTS)))
        native = write(self.root / "fake-native-cc", "#!" + PYTHON + " -I\n" + E4_NATIVE)
        for path in (cargo, native):path.chmod(0o700)
        inventory["cargo"]["sha256"] = sha(cargo)
        inventory["generators"]["CC"]["sha256"] = sha(native)
        body = self.tool.read_text()
        if case == "capture_fault":
            needle = 'try:dest=open(call/(name+".raw"),"xb")'; self.assertEqual(body.count(needle), 1)
            body = body.replace(needle, 'try:\n                if env.get("E4_CASE")=="capture_fault" and name=="stderr":raise OSError("TEST_CODE E4 capture")\n                dest=open(call/(name+".raw"),"xb")')
        if case == "forward_fault":
            needle = 'written = os.write(1 if stream == "stdout" else 2, view)'; self.assertEqual(body.count(needle), 1)
            body = body.replace(needle, 'if env.get("E4_CASE")=="forward_fault":raise OSError("TEST_CODE E4 forward")\n                            ' + needle)
        if case == "control_return":
            needle = 'receipt["controls_return"] = foreign_controls(session, policy, owner, context)'; self.assertEqual(body.count(needle), 1)
            body = body.replace(needle, needle + '\n            if env.get("E4_CASE")=="control_return":receipt["controls_return"]["session_owner_sha256"]="0"*64')
        if case == "fd_return":
            needle = 'receipt["tool_result"] = code; receipt["failures"].extend(faults)'; self.assertEqual(body.count(needle), 1)
            body = body.replace(needle, needle + '\n        if env.get("E4_CASE")=="fd_return":os.close(fds[0])')
        if case == "parallel":
            needle = '    atomic_json(call / "request.json", request)'; self.assertEqual(body.count(needle), 1)
            pause = '\n    if env.get("E4_CASE")=="parallel" and cwd.name in FOREIGN_E_ONLY_PACKAGES and Path(args[-1]).name=="1detect_compiler_family.c":\n        import time\n        (session/("e4-pending-"+cwd.name)).write_text(call.name)\n        deadline=time.monotonic()+10\n        while not (session/("e4-release-"+cwd.name)).exists():\n            require(time.monotonic()<deadline,"TEST_CODE E4 pending release");time.sleep(0.01)\n'
            body = body.replace(needle, pause + needle)
        self.tool.write_text(body); inventory["owner_sha256"] = sha(self.tool)
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly", "profile": owner.BUNDLED_PROFILE, "inventory": inventory}))
        return inventory


    def native_e4_result(self, case="normal"):
        inventory = self.prepare_native_e4(case)
        run = subprocess.run([PYTHON, "-I", str(self.tool), "record"], env=dict(os.environ),
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120)
        self.assertEqual(run.returncode, 2, run.stdout.decode(errors="replace") + run.stderr.decode(errors="replace"))
        record = self.record_result(run); session = Path(json.loads(run.stdout)["record_path"]).parent
        self.assertEqual(record["cargo_exit_code"], 0,
                         (session / "cargo.stderr.raw").read_text(errors="replace"))
        foreign = json.loads((session / "foreign-native-record.json").read_bytes())
        calls = [(p.parent, json.loads(p.read_bytes())) for p in (session / "foreign-native-invocations").glob("*/receipt.json")]
        self.assertEqual(record["foreign_native_record_sha256"], sha(session / "foreign-native-record.json"))
        self.assertEqual({r["name"] for r in json.loads((session / "foreign-forwarded.json").read_bytes())}, set(E4_FACTS))
        self.assertEqual((foreign["native_producer_qualification"], foreign["artifact_selection"]), ("not_issued", "not_observed"))
        self.assertEqual(record["selected_library"], [])
        self.assertTrue(all(r.get("operation", {}).get("class") in (None, "CompilerFamilyFileProbe") for _, r in calls))
        return inventory, session, record, foreign, calls


    def native_e4_private_negative_result(self, case):
        # Six fixed vendor-drift cuts: real CLI refusal plus a real negative tuple.
        self.assertIn(case, ("pin_pre", "pin_post", "copy_declaration_retained_0", "copy_declaration_retained_1",
                             "copy_declaration_request_only_0", "copy_declaration_request_only_1"))
        from unittest import mock
        inventory = self.prepare_native_e4(case)
        before = set((self.root / ".replay-build-records").glob("pending-*"))
        run = subprocess.run([PYTHON, "-I", str(self.tool), "record"], env=dict(os.environ),
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120)
        self.assertEqual(run.returncode, 2, run.stderr.decode(errors="replace")); self.assertEqual(run.stdout, b"")
        self.assertEqual(json.loads(run.stderr), {"schema": owner.SCHEMA, "state": "RecordingOnly", "reason": "Refused", "detail": "InventoryMismatch"})
        pending = set((self.root / ".replay-build-records").glob("pending-*")) - before
        self.assertEqual(len(pending), 1); session = pending.pop()
        self.assertFalse((session / "record.json").exists()); self.assertFalse((session / "foreign-native-record.json").exists())
        forwarded = session / "foreign-forwarded.json"
        self.assertTrue(forwarded.is_file(), (session / "cargo.stderr.raw").read_text(errors="replace"))
        self.assertEqual({r["name"] for r in json.loads(forwarded.read_bytes())}, set(E4_FACTS))
        calls = [(p.parent, json.loads(p.read_bytes())) for p in (session / "foreign-native-invocations").glob("*/receipt.json")]
        with mock.patch.object(owner, "POLICY", self.policy):
            privateNegativeObservation = owner.foreign_evidence_namespace(session)
        return inventory, session, calls, privateNegativeObservation


    def test_native_e4_observed_package_e_groups_and_retirement(self):
        for case in ("normal", "parallel"):
            with self.subTest(case=case):
                _, session, record, foreign, calls = self.native_e4_result(case)
                self.assertEqual(record["cargo_exit_code"], 0); self.assertEqual(len(calls), 44)
                self.assertEqual(foreign["stage"], "StageAIncomplete")
                self.assertIn("compiler-family-successors", foreign["unclosed"])
                self.assertFalse(any(b.startswith("ForeignOperation:") for b in foreign["blockers"]), foreign["blockers"])
                forwarded = json.loads((session / "foreign-forwarded.json").read_bytes())
                hits = [json.loads(p.read_bytes()) for p in (session / "foreign-entry").iterdir()]
                self.assertEqual(len(hits), 44)
                for name, (version, components, features, links, count) in E4_FACTS.items():
                    group = [(c, r) for c, r in calls if Path(r["context"]["manifest"]).name == name]
                    self.assertEqual(len(group), count)
                    retained_group = next(g for g in foreign["context_groups"] if g["package_id"].endswith("#" + name + "@" + version))
                    self.assertEqual(retained_group["observed_classes"], ["CompilerFamilyFileProbe"])
                    self.assertEqual(retained_group["missing_observation_classes"], ["CompilerFamilyHelpProbe", "CompilerFamilyVersionProbe"])
                    self.assertEqual((retained_group["per_probe_family_predecessor"], retained_group["family_qualification"]), ("not_observed", "not_issued"))
                    for call, r in group:
                        env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in r["environment_hex"].items()}
                        self.assertEqual([env["CARGO_PKG_VERSION_" + k] for k in ("MAJOR", "MINOR", "PATCH", "PRE")], list(components))
                        self.assertEqual((env["CARGO_PKG_VERSION"], env["CARGO_MANIFEST_LINKS"], env["CARGO_CFG_FEATURE"]), (version, links, ",".join(features)))
                        self.assertNotIn("LC_ALL", env); self.assertNotIn("ZERO_AR_DATE", env); self.assertEqual(env["LC_CTYPE"], "C.UTF-8")
                        self.assertEqual((r["protocol_state"], r["tool_result"], r["failures"]), ("Completed", 0, []))
                        self.assertEqual((call / "input-pre.raw").read_bytes(), D1_PROBE)
                        self.assertEqual(owner.native_state_key(r["input_pre"]), owner.native_state_key(r["input_post"]))
                        self.assertEqual(r["jobserver_identity"], r["jobserver_return"]); self.assertEqual(len(r["jobserver_identity"]["endpoints"]), 2)
                        self.assertIsNone(r["operation"]["predecessor"]); self.assertFalse(r["operation"]["retry"])
                        self.assertEqual(r["e_only_input_declaration"], {"state": "DeclaredOnly", "source_sha256":
                            {m: owner.FOREIGN_E_ONLY_SOURCE_PINS[m] for m in owner.FOREIGN_E_ONLY_INPUTS[name]}})
                        self.assertTrue(r["source_semantics"]["effective_stdout"]); self.assertTrue(r["source_semantics"]["markers"]["clang"])
                        item = next(o for o in foreign["operations"] if o["operation_id"] == call.name)
                        self.assertEqual(item["input_final_state"], "RetiredAfterCcReturn"); self.assertFalse(Path(r["operation"]["source"]).exists())
                        raw = [os.fsdecode(bytes.fromhex(v)) for v in r["args_hex"]]
                        forward = next(f for f in forwarded if f["name"] == name and f["args"] == raw)
                        hit = next(h for h in hits if h["cwd"] == r["context"]["manifest"] and h["argv"] == raw)
                        self.assertEqual(forward["status"], 0); self.assertTrue(hit["stdin_eof"])
                        self.assertEqual(hit["fds"], [e["fd"] for e in r["jobserver_identity"]["endpoints"]])
                        self.assertEqual(hit["inodes"], [e["inode"] for e in r["jobserver_identity"]["endpoints"]])
                        for stream in ("stdout", "stderr"):
                            self.assertEqual((call / (stream + ".raw")).read_bytes().hex(), forward[stream + "_hex"])
                            self.assertEqual(sha(call / (stream + ".raw")), r[stream + "_sha256"])
                    if case == "parallel":
                        pending = session / ("e4-pending-" + name); release = session / ("e4-release-" + name)
                        self.assertTrue(pending.is_file()); self.assertTrue(release.is_file())
                        pending_id = pending.read_text(); self.assertIn(pending_id, {c.name for c, _ in group})
                        self.assertEqual(json.loads((session / ("e4-observed-" + name)).read_bytes()),
                                         {"operation_id": pending_id, "request_absent": True, "receipt_absent": True})
                        released = json.loads(release.read_bytes())
                        release_call, release_receipt = next((c, r) for c, r in group if c.name == released["operation_id"])
                        self.assertNotEqual(release_call.name, pending_id); self.assertEqual(released["receipt_sha256"], sha(release_call / "receipt.json"))
                        self.assertEqual((release_receipt["protocol_state"], release_receipt["tool_result"], release_receipt["failures"]), ("Completed", 0, []))
                        self.assertTrue(release_receipt["operation"]["source"].endswith("/2detect_compiler_family.c"))
                literal = str(session / "vendor" / owner.PROBE_LITERAL)
                self.assertTrue(any(c["path"] == literal for c in record["consumed_sources"]))
                self.assertFalse(any(c["path"] in foreign["quarantine"]["paths"] for c in record["consumed_sources"] if c["path"] != literal))


    def test_native_e4_context_admission_and_genuine_warning_retry(self):
        rejected = {"name": "FixedPackageContext", "version": "FixedPackageContext", "labels_absent": "FixedPackageContext",
            "component_major": "ForeignVersion", "component_minor": "ForeignVersion", "component_patch": "ForeignVersion", "component_pre": "ForeignVersion",
            "links": "ForeignFeaturesLinks", "features": "ForeignFeaturesLinks", "manifest": "ForeignManifestOutDir",
            "out": "FixedPackageOutDir", "out_label": "ForeignManifestOutDir", "target": "ForeignConfiguration",
            "windows": "ForeignSourceBranch", "pkgconfig": "ForeignSourceBranch", "locale": "ForeignProbeEnvironment",
            "flags": "NativeEnvironmentInjection:CFLAGS", "fd_missing": "ForeignJobserverRequired", "fd_reversed": "InvalidJobserverDescriptors",
            "fd_foreign": "NativeJobserverPair", "literal": "ForeignProbeLiteral", "extent": "NativeInputExtent", "hardlink": "NativeFileAlias",
            "symlink": "ForeignProbePath", "overflow": "ForeignProbePath", "unknown": "ForeignSourceContext", "compile": "ForeignEOnlyArgv",
            "archive": "ForeignEOnlyArgv", "help": "ForeignEOnlyArgv", "version_probe": "ForeignEOnlyArgv", "extra": "ForeignEOnlyArgv", "retry": "ForeignProbePredecessor"}
        for case, marker in rejected.items():
            with self.subTest(case=case):
                _, session, _, foreign, calls = self.native_e4_result("reject_" + case)
                self.assertEqual(len(calls), 2); self.assertFalse((session / "foreign-entry").exists())
                self.assertTrue(all(r["protocol_state"] == "ProtocolRefused" and r["tool_result"] is None
                                    and any(marker in f for f in r["failures"]) for _, r in calls), calls)
                self.assertTrue(any("ForeignProtocolSticky" in b for b in foreign["blockers"]))
        for case in ("warning_stdout", "warning_stderr"):
            with self.subTest(case=case):
                _, _, _, foreign, calls = self.native_e4_result(case)
                self.assertEqual(len(calls), 4); by_id = {c.name: r for c, r in calls}
                self.assertFalse(any(b.startswith("ForeignOperation:") for b in foreign["blockers"]), foreign["blockers"])
                for _, r in calls:
                    self.assertEqual(r["protocol_state"], "Completed")
                    if r["operation"]["retry"]:
                        first = by_id[r["operation"]["predecessor"]]
                        self.assertEqual((first["tool_result"], r["tool_result"]), (3, 0))
                        self.assertEqual(first["operation"]["source"], r["operation"]["source"])
                        self.assertTrue(first["source_semantics"]["warning_retry_requested"]); self.assertTrue(r["source_semantics"]["effective_stdout"])
        for case, marker in (("duplicate", "ForeignProbePredecessor"), ("predecessor_tamper", "ForeignSourceSemantics")):
            with self.subTest(case=case):
                _, session, _, foreign, calls = self.native_e4_result(case)
                self.assertEqual(len(calls), 4); self.assertEqual(len(list((session / "foreign-entry").iterdir())), 2)
                later = [r for _, r in calls if r["tool_result"] is None]
                self.assertEqual(len(later), 2); self.assertTrue(all(marker in r["failures"] for r in later), later)
                self.assertTrue(any("ForeignOperation:" in b for b in foreign["blockers"]))
        _, session, calls, observation = self.native_e4_private_negative_result("pin_pre")
        self.assertEqual(len(calls), 2); self.assertFalse((session / "foreign-entry").exists())
        self.assertTrue(all((r["protocol_state"], r["tool_result"]) == ("ProtocolRefused", None)
                            and "ForeignEOnlySourcePin" in r["failures"] for _, r in calls), calls)
        paths, probes, hashes, blockers = observation
        for _, r in calls:
            name = Path(os.fsdecode(bytes.fromhex(r["cwd_hex"]))).name
            header = session / "vendor" / name / ("liblz4/lib/lz4.h" if name == "lz4-sys" else "zstd/lib/common/allocations.h")
            self.assertEqual(r["e_only_input_declaration"]["state"], "DeclaredOnly")
            self.assertIn(str(header), paths); self.assertIn(sha(header), hashes)
            self.assertIn(owner.FOREIGN_E_ONLY_SOURCE_PINS[str(header.relative_to(session / "vendor"))], hashes)


    def test_native_e4_sticky_raw_faults_and_negative_ownership(self):
        for case, code in (("nonzero", 7), ("signal", -15)):
            with self.subTest(case=case):
                _, session, _, foreign, calls = self.native_e4_result(case)
                self.assertEqual(len(calls), 2)
                for call, r in calls:
                    self.assertEqual((r["protocol_state"], r["tool_result"], r["failures"]), ("Completed", code, []))
                    self.assertFalse(r["source_semantics"]["effective_stdout"])
                    forward = next(f for f in json.loads((session / "foreign-forwarded.json").read_bytes()) if f["name"] == Path(r["context"]["manifest"]).name)
                    self.assertEqual(forward["status"], code if code >= 0 else 128 - code)
                    for stream in ("stdout", "stderr"):self.assertEqual((call / (stream + ".raw")).read_bytes().hex(), forward[stream + "_hex"])
        faults = {"capture_fault": "ForeignCaptureSticky", "forward_fault": "ForeignForward:", "fd_return": "InvalidJobserverDescriptors",
                  "post_missing": "NativeInputMissing", "post_change": "ForeignInputChanged", "control_post": "ForeignControlChanged", "control_return": "ForeignControlChanged",
                  "snapshot_bad": "NativeSnapshotChanged", "stream_bad": "ForeignStreamChanged", "orphan": "ForeignOperation:"}
        for case, marker in faults.items():
            with self.subTest(case=case):
                _, session, record, foreign, calls = self.native_e4_result(case)
                self.assertEqual(record["cargo_exit_code"], 0); self.assertEqual(len(calls), 2)
                if case not in ("snapshot_bad", "stream_bad", "orphan"):
                    self.assertTrue(all(r["protocol_state"] == "ProtocolRefused" and r["tool_result"] == 0 for _, r in calls), calls)
                    self.assertTrue(all(any(marker in f for f in r["failures"]) for _, r in calls), calls)
                    for call, r in calls:
                        self.assertEqual(r["operation_id"], call.name)
                        self.assertIn("ForeignOperation:" + call.name + ":ForeignProtocolSticky", foreign["blockers"])
                        self.assertEqual(sha(call / "stdout.raw"), r["stdout_sha256"])
                        if case == "post_change":
                            self.assertEqual(r["input_pre"]["sha256"], owner.PROBE_DIGEST)
                            self.assertNotEqual(r["input_pre"]["sha256"], r["input_post"]["sha256"])
                            self.assertIn(r["input_post"]["sha256"], foreign["quarantine"]["sha256"])
                else:
                    self.assertTrue(all(r["protocol_state"] == "Completed" for _, r in calls))
                    if case == "orphan":
                        orphan = session / "foreign-native-invocations" / ("f" * 32)
                        self.assertTrue(orphan.is_dir()); self.assertFalse(orphan.is_symlink())
                        self.assertEqual(list(orphan.iterdir()), [])
                        self.assertTrue(any(b.startswith("ForeignOperation:" + orphan.name + ":") for b in foreign["blockers"]))
                    else:
                        for call, r in calls:
                            self.assertEqual(r["operation_id"], call.name)
                            self.assertIn("ForeignOperation:" + call.name + ":" + marker, foreign["blockers"])
                            raw = call / ("input-post.raw" if case == "snapshot_bad" else "stdout.raw")
                            retained = r["input_post"]["sha256"] if case == "snapshot_bad" else r["stdout_sha256"]
                            self.assertNotEqual(sha(raw), retained)
                            self.assertIn(str(raw), foreign["quarantine"]["paths"])
                            self.assertIn(sha(raw), foreign["quarantine"]["sha256"])
                            self.assertIn(retained, foreign["quarantine"]["sha256"])
        for index in (0, 1):
            controls = (("stdout", "source", "current"), ("stderr", "extern", "current"), ("stdout", "output", "current"),
                        ("stdout", "source", "retained_corrupt"), ("stderr", "extern", "retained_missing"), ("stdout", "source", "request_only"))
            for stream, role, cut in controls:
                case = "stream_" + str(index) + "_" + stream + "_" + role + "_" + cut
                with self.subTest(case=case):
                    _, session, record, foreign, calls = self.native_e4_result(case)
                    control = json.loads((session / "e4-copy-control.json").read_bytes()); copy = Path(control["copy"])
                    self.assertEqual(copy.read_bytes().hex(), control["body_hex"]); self.assertEqual(sha(copy), control["sha256"])
                    self.assertIn(sha(copy), foreign["quarantine"]["sha256"])
                    cc_path = session / "invocations" / control["cc_id"] / "receipt.json"; cc = json.loads(cc_path.read_bytes())
                    self.assertEqual(cc["source"], str(session / "vendor/cc/src/lib.rs")); self.assertEqual(sha(cc_path), control["cc_receipt_sha256"])
                    bound = [r for r in record["invocations"] if r["invocation_id"] == control["cc_id"]]
                    self.assertEqual(len(bound), 1); self.assertEqual(bound[0]["receipt_sha256"], sha(cc_path))
                    self.assertIn("NativeOutputRole:" + (control["cc_id"] if role == "output" else str(copy)), record["blockers"])
                    self.assertFalse(any(c["path"] == str(copy) for c in record["consumed_sources"]))
                    self.assertFalse(any(e["path"] == str(copy) and e["producers"] for e in record["extern_edges"]))
                    literal = str(session / "vendor" / owner.PROBE_LITERAL)
                    self.assertEqual(sha(Path(literal)), owner.PROBE_DIGEST)
                    if role == "output":
                        self.assertIn({"path": str(copy), "kind": "link"}, cc["declared_outputs"])
                        dep = next(o for o in cc["outputs"] if o["kind"] == "dep-info")
                        self.assertIn(literal, dep["dep_info"]["paths"]); self.assertEqual(sha(Path(dep["path"])), dep["sha256"])
                        self.assertIn(os.fsencode(literal), Path(dep["path"]).read_bytes())
                        self.assertIn("UnresolvedCargoArtifact:" + cc["source"], record["blockers"])
                        self.assertFalse(any(c["path"] == literal for c in record["consumed_sources"]))
                        self.assertFalse(any(control["cc_id"] in e["producers"] for e in record["extern_edges"]))
                        self.assertFalse(any(a["producer_invocation"] == control["cc_id"] for a in record["build_script_associations"]))
                    else:self.assertTrue(any(c["path"] == literal for c in record["consumed_sources"]))
                    call = session / "foreign-native-invocations" / control["operation_id"]
                    if cut.startswith("retained_"):
                        self.assertNotIn(sha(copy), {sha(q) for q in (session / "foreign-native-invocations").glob("*/*.raw")})
                        self.assertEqual(json.loads((call / "receipt.json").read_bytes())[stream + "_sha256"], sha(copy))
                        self.assertTrue(any(b.startswith("ForeignNamespace:" + call.name + ":") for b in foreign["blockers"]))
                    if cut == "request_only":
                        self.assertTrue((call / "request.json").is_file()); self.assertFalse((call / "receipt.json").exists())
                        self.assertEqual(sha(call / (stream + ".raw")), sha(copy))
                        self.assertTrue(any(b.startswith("ForeignOperation:" + call.name + ":") for b in foreign["blockers"]))
        _, session, record, foreign, calls = self.native_e4_result("flag_negative")
        control = json.loads((session / "e4-copy-control.json").read_bytes()); copy = Path(control["copy"])
        self.assertEqual(copy.read_bytes(), b"int main(void) { return 0; }"); self.assertEqual(copy.stat().st_size, 28)
        self.assertEqual(sha(copy), owner.FOREIGN_FLAG_LITERAL_DIGEST); self.assertIn(sha(copy), foreign["quarantine"]["sha256"])
        flag = [r for _, r in calls if "-c" in [os.fsdecode(bytes.fromhex(v)) for v in r["args_hex"]]]
        self.assertEqual(len(flag), 1); self.assertEqual((flag[0]["protocol_state"], flag[0]["tool_result"], flag[0]["failures"]), ("ProtocolRefused", None, ["ForeignSourceContext"]))
        self.assertEqual(flag[0]["e_only_input_declaration"]["state"], "DeclaredOnly")
        self.assertIn("NativeOutputRole:" + str(copy), record["blockers"])
        self.assertFalse(any(c["path"] == str(copy) for c in record["consumed_sources"]))
        self.assertEqual(len(list((session / "foreign-entry").iterdir())), 2)
        for case in ("pin_post", "copy_declaration_retained_0", "copy_declaration_retained_1", "copy_declaration_request_only_0", "copy_declaration_request_only_1"):
            with self.subTest(case=case):
                _, session, calls, observation = self.native_e4_private_negative_result(case)
                paths, probes, hashes, blockers = observation
                self.assertEqual(len(list((session / "foreign-entry").iterdir())), 2)
                if case == "pin_post":
                    self.assertEqual(len(calls), 2)
                    self.assertTrue(all(r["protocol_state"] == "ProtocolRefused" and r["tool_result"] == 0
                                        and "ForeignEOnlySourcePin" in r["failures"] for _, r in calls), calls)
                    for call, r in calls:
                        self.assertEqual((call / "input-pre.raw").read_bytes(), D1_PROBE)
                        self.assertEqual(sha(call / "stdout.raw"), r["stdout_sha256"])
                        header = Path(next(os.fsdecode(bytes.fromhex(v)) for k, v in r["environment_hex"].items() if os.fsdecode(bytes.fromhex(k)) == "E4_HEADER"))
                        self.assertIn(str(header), paths); self.assertIn(sha(header), hashes)
                else:
                    control = json.loads((session / "e4-declaration-copy.json").read_bytes()); copy = Path(control["copy"])
                    self.assertEqual(copy.read_bytes().hex(), control["body_hex"]); self.assertIn(str(Path(control["header"])), paths)
                    self.assertIn(sha(copy), hashes); self.assertTrue(owner.foreign_owned(str(copy), session, paths, hashes))
                    call = session / "foreign-native-invocations" / control["operation_id"]
                    if control["kind"] == "retained":
                        self.assertTrue(Path(control["header"]).is_file()); self.assertNotEqual(sha(Path(control["header"])), sha(copy)); self.assertTrue((call / "receipt.json").is_file())
                        self.assertEqual(sha(copy), control["expected_sha256"])
                        self.assertNotIn(sha(copy), {sha(q) for q in (session / "foreign-native-invocations").glob("*/*.raw")})
                        self.assertTrue(any(b.startswith("ForeignNamespace:" + call.name + ":") for b in blockers))
                        if control["name"] == "zstd-sys":
                            retained = json.loads((call / "receipt.json").read_bytes())["e_only_input_declaration"]["source_sha256"]
                            self.assertEqual(next(iter(retained)), "\x00TEST_CODE_bad_member")
                            self.assertTrue(set(retained.values()) <= hashes)
                            self.assertIn("ForeignNamespace:" + call.name + ":ForeignEOnlyDeclarationPath:ValueError", blockers)
                    else:
                        self.assertTrue((call / "request.json").is_file()); self.assertFalse((call / "receipt.json").exists())
                        self.assertNotEqual(sha(copy), control["expected_sha256"])
                        self.assertIn(control["expected_sha256"], hashes)


    def prepare_native_e5(self, case="follow_normal"):
        # Fresh private canonical geometry; old E2/E3 roots/fixtures are untouched.
        temp = tempfile.TemporaryDirectory(prefix="TEST_CODE_followups_", dir="/tmp")
        self.addCleanup(temp.cleanup)
        base = Path(temp.name).resolve()
        ring_names = [basename for _, basename in E3_SELECTED["ring"]] + list(owner.FOREIGN_ARCHIVE_REMAINING_MEMBERS)
        chosen = None
        for count in range(200):
            root = base if count == 0 else base / ("geometry_" + "p" * count)
            out = root / ".replay-build-records" / ("pending-" + "0" * 32) / "target" / owner.TARGET / "debug/build/ring-0123456789abcdef/out"
            paths = [os.fsencode(str(out / n)) for n in ring_names]
            if sum(map(len, paths[:16])) <= 4000 < sum(map(len, paths[:17])) and sum(map(len, paths[16:])) <= 4000:
                chosen = root; break
        self.assertIsNotNone(chosen, "No canonical private root meets the genuine byte interval")
        self.root = base / ("wrong_geometry_" + "p" * 180) if case == "follow_geometry" else chosen
        self.root.mkdir(parents=True, exist_ok=True)
        self.tool = self.root / "tools/replay_build_owner_v1.py"; self.tool.parent.mkdir()
        shutil.copyfile(TOOL, self.tool); self.policy = self.tool.with_name("replay_build_pin_v1_manifest.json")
        inventory = self.prepare_native_e2("normal")
        selected = {"ring": E3_SELECTED["ring"] + [next(row for row in E2_SOURCE_DATA["ring"] if row[1] == name)
                    for name in owner.FOREIGN_ARCHIVE_REMAINING_MEMBERS], "psm": E3_SELECTED["psm"]}
        cargo_path = self.root / "fake-cargo"; body = cargo_path.read_text()
        for old, new in (("CASE='normal';argv=", "CASE=" + repr(case) + ";argv="),
                         ("    selected=E2_SOURCE_DATA[name] if CASE in ('normal','parallel') else E2_SOURCE_DATA[name][:2 if CASE=='sticky' else 1]", "    selected=E5_SELECTED[name]"),
                         ("import hashlib,json,os,pathlib,subprocess,sys,threading", "import hashlib,json,os,pathlib,subprocess,sys,threading\nE5_SELECTED=" + repr(selected)),
                         ("        for specification in selected:object_call(specification)", "        for specification in selected:object_call(specification)\n" + E5_ARCHIVE_CARGO)):
            self.assertEqual(body.count(old), 1); body = body.replace(old, new)
        old = "object_paths=[p for p in paths if json.loads(p.read_text()).get('operation',{}).get('class')=='CompilerObjectCompile'"
        new = "object_paths=[p for p in paths if isinstance(json.loads(p.read_text()).get('operation'),dict) and json.loads(p.read_text())['operation'].get('class')=='CompilerObjectCompile'"
        self.assertEqual(body.count(old), 1);body = body.replace(old, new)
        if case.startswith("follow_copy_"):
            copy_body = E3_COPY_CARGO.replace("ar_copy_", "follow_copy_")
            old = "    p=next(p for p in paths if json.loads(p.read_text()).get('operation',{}).get('class')=='ArchiverFirstAppend'\n           and json.loads(p.read_text())['context']['manifest']==str(session/'vendor/ring'))"
            new = "    matches=[p for p in paths if json.loads(p.read_text()).get('operation',{}).get('class')=='ArchiverIndex' and json.loads(p.read_text())['context']['manifest']==str(session/'vendor/psm')];assert len(matches)==1;p=matches[0]"
            self.assertEqual(copy_body.count(old), 1); copy_body = copy_body.replace(old, new)
            old = "emit({'reason':'build-finished','success':True})"; self.assertEqual(body.count(old), 1)
            body = body.replace(old, copy_body + "\n" + old)
        cargo_path.write_text(body)
        ar = write(self.root / "fake-native-ar", "#!" + PYTHON + " -I\n" + E5_NATIVE_AR); ar.chmod(0o700)
        inventory["cargo"]["sha256"] = sha(cargo_path)
        inventory["generators"]["AR"] = {"path": str(ar), "sha256": sha(ar)}
        body = self.tool.read_text()
        if case == "follow_index_capture":
            old = 'try:dest=open(call/(name+".raw"),"xb")';self.assertEqual(body.count(old), 1)
            body = body.replace(old, 'try:\n                if env.get("E1_CASE")=="follow_index_capture" and argv[1:2]==["s"] and name=="stderr":raise OSError("TEST_CODE index capture")\n                dest=open(call/(name+".raw"),"xb")')
        if case == "follow_index_forward":
            old = 'written = os.write(1 if stream == "stdout" else 2, view)';self.assertEqual(body.count(old), 1)
            body = body.replace(old, 'if archiving and operation["class"]=="ArchiverIndex":raise OSError("TEST_CODE index forward")\n                            ' + old)
        if case == "follow_index_fd_return":
            old = 'receipt["tool_result"] = code; receipt["failures"].extend(faults)';self.assertEqual(body.count(old), 1)
            body = body.replace(old, old + '\n        if archiving and operation["class"]=="ArchiverIndex":os.close(fds[0])')
        self.tool.write_text(body); inventory["owner_sha256"] = sha(self.tool)
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly", "profile": owner.BUNDLED_PROFILE, "inventory": inventory}))
        return inventory

    def native_e5_result(self, case="follow_normal"):
        inventory = self.prepare_native_e5(case)
        run = subprocess.run([PYTHON, "-I", str(self.tool), "record"], env=dict(os.environ),
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120)
        self.assertEqual(run.returncode, 2, run.stdout.decode(errors="replace") + run.stderr.decode(errors="replace"))
        record = self.record_result(run);session = Path(json.loads(run.stdout)["record_path"]).parent
        self.assertEqual(record["cargo_exit_code"], 0, (session / "cargo.stderr.raw").read_text(errors="replace"))
        foreign = json.loads((session / "foreign-native-record.json").read_bytes())
        self.assertEqual(record["foreign_native_record_sha256"], sha(session / "foreign-native-record.json"))
        self.assertEqual((foreign["stage"], foreign["native_producer_qualification"], foreign["artifact_selection"]),
                         ("StageCIncomplete", "not_issued", "not_observed"))
        self.assertEqual(record["selected_library"], [])
        calls = {p.parent.name: (p.parent, json.loads(p.read_bytes())) for p in (session / "foreign-native-invocations").glob("*/receipt.json")}
        controls = json.loads((session / "followups-control.json").read_bytes())
        return inventory, session, record, foreign, calls, controls

    def native_e5_source_negative_result(self):
        from unittest import mock
        inventory = self.prepare_native_e5("follow_index_source_post")
        pending = self.root / ".replay-build-records";before = set(pending.glob("pending-*"))
        run = subprocess.run([PYTHON, "-I", str(self.tool), "record"], env=dict(os.environ),
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120)
        self.assertEqual(run.returncode, 2);self.assertEqual(run.stdout, b"")
        self.assertEqual(json.loads(run.stderr), {"schema": owner.SCHEMA, "state": "RecordingOnly",
                                                 "reason": "Refused", "detail": "InventoryMismatch"})
        created = set(pending.glob("pending-*")) - before;self.assertEqual(len(created), 1);session = created.pop()
        self.assertFalse((session / "record.json").exists());self.assertFalse((session / "foreign-native-record.json").exists())
        forwarded = json.loads((session / "foreign-forwarded.json").read_bytes())
        self.assertEqual({r["name"] for r in forwarded}, {"ring", "psm"})
        calls = {p.parent.name: (p.parent, json.loads(p.read_bytes())) for p in (session / "foreign-native-invocations").glob("*/receipt.json")}
        with mock.patch.object(owner, "POLICY", self.policy):
            privateNegativeObservation = owner.foreign_evidence_namespace(session)
        return inventory, session, calls, privateNegativeObservation

    def test_native_e5_fixed_remaining_and_index_chain_geometry(self):
        for case in ("follow_normal", "follow_partial"):
            with self.subTest(case=case):
                inventory, session, record, foreign, calls, controls = self.native_e5_result(case)
                self.assertEqual(len(controls), 6);self.assertEqual(len(list((session / "archive-entry").iterdir())), 6)
                forwarded = json.loads((session / "foreign-forwarded.json").read_bytes())
                self.assertFalse(any(b.startswith("ForeignOperation:") for b in foreign["blockers"]), foreign["blockers"])
                geometry = next(g for g in json.loads((session / "followups-geometry.json").read_bytes()) if g["name"] == "ring")
                self.assertLessEqual(geometry["first_bytes"], 4000);self.assertGreater(geometry["first_plus_next_bytes"], 4000);self.assertLessEqual(geometry["tail_bytes"], 4000)
                for name in ("ring", "psm"):
                    rows = [c for c in controls if c["name"] == name];self.assertEqual([c["stage"] for c in rows], ["FirstD", "FirstFallbackCQ", "RingRemainingCQ" if name == "ring" else "PSMIndexS"])
                    for index, control in enumerate(rows):
                        call, receipt = calls[control["operation_id"]]
                        self.assertEqual((receipt["protocol_state"], receipt["failures"]), ("Completed", []))
                        self.assertEqual(receipt["tool_result"], (7 if case == "follow_partial" else 1) if index == 0 else 0)
                        self.assertEqual(receipt["tool_sha256"], inventory["generators"]["AR"]["sha256"])
                        self.assertEqual(receipt["family_pre"], receipt["family_return"]);self.assertEqual(receipt["jobserver_identity"], receipt["jobserver_return"])
                        self.assertEqual(receipt["archive_member_producers_pre"], receipt["archive_member_producers_return"])
                        self.assertEqual(receipt["archive_history_pre"], [{"operation_id": c["operation_id"], "request_sha256": sha(calls[c["operation_id"]][0] / "request.json"), "receipt_sha256": sha(calls[c["operation_id"]][0] / "receipt.json")} for c in rows[:index]])
                        self.assertEqual(receipt["archive_history_pre"], receipt["archive_history_return"])
                        for pre, post, producer in zip(receipt["archive_members_pre"], receipt["archive_members_post"], receipt["archive_member_producers_pre"]):
                            self.assertEqual(pre["sha256"], post["sha256"]);self.assertEqual(pre["sha256"], producer["sha256"])
                            cc = calls[producer["operation_id"]];self.assertEqual((cc[1]["protocol_state"], cc[1]["tool_result"]), ("Completed", 0))
                            self.assertEqual(producer["request_sha256"], sha(cc[0] / "request.json"));self.assertEqual(producer["receipt_sha256"], sha(cc[0] / "receipt.json"))
                            for state in (pre, post):self.assertEqual(sha(call / state["snapshot"]), state["sha256"])
                        observed = [r for r in forwarded if r["name"] == name and r["args"] == control["args"]]
                        self.assertEqual(len(observed), 1);self.assertEqual(observed[0]["status"], receipt["tool_result"])
                        for flow in ("stdout", "stderr"):
                            self.assertEqual((call / (flow + ".raw")).read_bytes(), ("TEST_CODE_followup_" + flow + "\n").encode())
                            self.assertEqual(sha(call / (flow + ".raw")), receipt[flow + "_sha256"])
                            self.assertEqual(observed[0][flow + "_hex"], (call / (flow + ".raw")).read_bytes().hex())
                        hit = next(json.loads(p.read_bytes()) for p in (session / "archive-entry").iterdir() if json.loads(p.read_bytes())["argv"] == control["args"] and json.loads(p.read_bytes())["cwd"] == receipt["context"]["manifest"])
                        self.assertTrue(hit["stdin_eof"]);self.assertEqual(hit["fds"], [p["fd"] for p in receipt["jobserver_identity"]["endpoints"]]);self.assertEqual(hit["inodes"], [p["inode"] for p in receipt["jobserver_identity"]["endpoints"]])
                        self.assertEqual(receipt["archive_operand_ledger_return"]["archive_member_inventory"], "not_observed")
                        if index:
                            previous = calls[rows[index-1]["operation_id"]][1]
                            self.assertEqual(receipt["archive_predecessor"], rows[index-1]["operation_id"])
                            self.assertEqual({k:v for k,v in receipt["archive_pre"].items() if k != "snapshot"}, {k:v for k,v in previous["archive_post"].items() if k != "snapshot"})
                        if receipt["archive_post"]["exists"]:self.assertEqual(sha(call / receipt["archive_post"]["snapshot"]), receipt["archive_post"]["sha256"])
                    last = calls[rows[-1]["operation_id"]][1]
                    ledger = last["archive_operand_ledger_return"]["producers"]
                    self.assertEqual(len(ledger), 29 if name == "ring" else 1);self.assertEqual(len({p["path"] for p in ledger}), len(ledger))
                    self.assertEqual(len(last["archive_member_producers_pre"]), 13 if name == "ring" else 1)
                    if name == "psm":
                        self.assertEqual(len(rows[-1]["args"]), 2);self.assertNotEqual(last["archive_pre"]["sha256"], last["archive_post"]["sha256"])

    def test_native_e5_stage_templates_history_and_once_refusals(self):
        controls_to_reason = {"follow_tail_prior_raw_out": "ForeignArchiveReceiptBinding", "follow_tail_prior_raw_cwd": "ForeignArchiveReceiptBinding",
            "follow_index_prior_raw_out": "ForeignArchiveReceiptBinding", "follow_index_prior_raw_cwd": "ForeignArchiveReceiptBinding",
            "follow_geometry": "ForeignArchiveBatchGeometry", "follow_tail_order": "ForeignArchiveTemplate",
            "follow_tail_duplicate_member": "ForeignArchiveTemplate", "follow_tail_readd": "ForeignArchiveTemplate",
            "follow_tail_d": "ForeignArchiveTemplate", "follow_tail_early": "ForeignArchivePredecessor",
            "follow_tail_repeat": "ForeignArchivePredecessor", "follow_index_early": "ForeignArchivePredecessor",
            "follow_index_member": "ForeignArchiveTemplate", "follow_index_d": "ForeignArchiveTemplate",
            "follow_archive_drift": "NativeVersionChanged", "follow_archive_recreate": "NativeVersionChanged",
            "follow_history_missing": "No such file", "follow_history_refused": "ForeignArchiveSticky",
            "follow_history_none": "ForeignArchiveOperationFields", "follow_history_producer": "ForeignArchiveMemberChanged",
            "follow_history_role": "ForeignFamilyReceiptBinding"}
        for case, reason in controls_to_reason.items():
            with self.subTest(case=case):
                _, session, _, foreign, calls, controls = self.native_e5_result(case)
                name = "psm" if case.startswith("follow_index_") else "ring"
                rows = [c for c in controls if c["name"] == name];last = rows[-1];call, receipt = calls[last["operation_id"]]
                self.assertEqual((receipt["protocol_state"], receipt["tool_result"]), ("ProtocolRefused", None))
                self.assertTrue(any(reason in failure for failure in receipt["failures"]), receipt)
                hits = [json.loads(p.read_bytes()) for p in (session / "archive-entry").iterdir()]
                attempted = [h for h in hits if h["cwd"] == receipt["context"]["manifest"]]
                self.assertEqual(len(attempted), 1 if case in ("follow_tail_early", "follow_index_early") else 3 if case == "follow_tail_repeat" else 2)
                self.assertFalse((call / "stdout.raw").exists());self.assertFalse((call / "stderr.raw").exists())
                self.assertTrue(any(b.startswith("ForeignOperation:" + call.name + ":") for b in foreign["blockers"]))
                if "_prior_raw_" in case:
                    poison = json.loads((session / "followups-poison-control.json").read_bytes())
                    self.assertEqual(poison["next_id"], call.name)
                    self.assertEqual(poison["archive_before_sha256"], poison["archive_after_sha256"])
                    self.assertEqual(sha(Path(poison["archive_path"])), poison["archive_before_sha256"])
                    self.assertEqual(poison["prefix_receipt_before_sha256"], poison["prefix_receipt_after_sha256"])
                    self.assertEqual(poison["ledger_before"], poison["ledger_after"])
                    self.assertEqual(len(poison["ledger_after"]["producers"]), 16 if name == "ring" else 1)
                    self.assertNotIn("archive_operand_ledger_pre", receipt)
                    prior_call, prior = calls[poison["operation_id"]]
                    self.assertEqual(sha(prior_call / "receipt.json"), poison["poison_receipt_sha256"])
                    self.assertEqual((prior["protocol_state"], prior["tool_result"], prior["failures"]), ("ProtocolRefused", None, ["ForeignArchiveTemplate"]))
                    self.assertEqual(prior["context"], receipt["context"])
                    raw = json.loads((prior_call / "request.json").read_bytes())
                    field = "environment_hex" if case.endswith("_raw_out") else "cwd_hex"
                    self.assertNotEqual(raw[field], prior[field]);self.assertEqual(raw["role"], prior["role"])
                    self.assertFalse((prior_call / "stdout.raw").exists());self.assertFalse((prior_call / "stderr.raw").exists())
                    self.assertIn("ForeignOperation:" + call.name + ":ForeignProtocolSticky", foreign["blockers"])
                    self.assertTrue(any(b.startswith("ForeignOperation:" + prior_call.name + ":") for b in foreign["blockers"]))
                    if name == "ring":
                        other = [c for c in controls if c["name"] == "psm"]
                        self.assertEqual([c["stage"] for c in other], ["FirstD", "FirstFallbackCQ", "PSMIndexS"])
                        other_receipt = calls[other[-1]["operation_id"]][1]
                        self.assertEqual((other_receipt["protocol_state"], other_receipt["tool_result"], other_receipt["failures"]), ("Completed", 0, []))
                        self.assertEqual(len([h for h in hits if h["cwd"] == other_receipt["context"]["manifest"]]), 3)
                        self.assertEqual(len(other_receipt["archive_operand_ledger_return"]["producers"]), 1)

    def test_native_e5_partial_sticky_and_ordinary_ownership_quarantine(self):
        for case, status in (("follow_tail_nonzero", 9), ("follow_index_nonzero", 11), ("follow_index_signal", -15)):
            with self.subTest(case=case):
                _, session, _, foreign, calls, controls = self.native_e5_result(case)
                name = "ring" if case == "follow_tail_nonzero" else "psm";last = [c for c in controls if c["name"] == name][-1];call, receipt = calls[last["operation_id"]]
                self.assertEqual((receipt["protocol_state"], receipt["tool_result"], receipt["failures"]), ("Completed", status, []))
                self.assertEqual(last["status"], 143 if status == -15 else status)
                self.assertTrue(receipt["archive_post"]["exists"]);self.assertEqual(sha(call / receipt["archive_post"]["snapshot"]), receipt["archive_post"]["sha256"])
                self.assertTrue(any(b == "ForeignArchiveUnresolvedNonzero:" + call.name for b in foreign["blockers"]))
                self.assertEqual(len(receipt["archive_operand_ledger_return"]["producers"]), 16 if name == "ring" else 1)
        faults = {"follow_index_input_post": "ForeignArchiveInputChanged", "follow_index_source_post": "ForeignCompileSourcePin",
                  "follow_index_capture": "ForeignCaptureSticky", "follow_index_forward": "ForeignForward", "follow_index_fd_return": "InvalidJobserverDescriptors"}
        for case, reason in faults.items():
            with self.subTest(case=case):
                if case == "follow_index_source_post":
                    _, session, calls, privateNegativeObservation = self.native_e5_source_negative_result()
                    paths, _, hashes, _ = privateNegativeObservation
                    matches = [(c, r) for c, r in calls.values() if isinstance(r.get("operation"), dict) and r["operation"]["class"] == "ArchiverIndex"]
                    self.assertEqual(len(matches), 1);call, receipt = matches[0]
                    self.assertEqual((receipt["protocol_state"], receipt["tool_result"]), ("ProtocolRefused", 0))
                    self.assertEqual(receipt["failures"], ["ForeignCompileSourcePin"])
                    self.assertTrue(receipt["archive_post"]["exists"]);self.assertEqual(len(receipt["archive_members_post"]), 1)
                    for state in [receipt["archive_pre"], receipt["archive_post"], *receipt["archive_members_post"]]:
                        snapshot = call / state["snapshot"];self.assertEqual(sha(snapshot), state["sha256"])
                        self.assertIn(state["sha256"], hashes);self.assertIn(state["path"], paths)
                    source = session / "vendor/psm/src/arch/x86_64.s"
                    self.assertEqual(source.read_bytes(), b"TEST_CODE_changed_index_source")
                    self.assertIn(str(source), paths);self.assertIn(sha(source), hashes)
                    self.assertNotIn("compile_pins_post", receipt)
                    for stream in ("stdout", "stderr"):
                        self.assertEqual((call / (stream + ".raw")).read_bytes(), ("TEST_CODE_followup_" + stream + "\n").encode())
                    self.assertEqual(len([p for p in (session / "archive-entry").iterdir() if json.loads(p.read_bytes())["stage"] == "PSMIndexS"]), 1)
                    continue
                _, session, _, foreign, calls, controls = self.native_e5_result(case)
                last = [c for c in controls if c["name"] == "psm"][-1];call, receipt = calls[last["operation_id"]]
                self.assertEqual((receipt["protocol_state"], receipt["tool_result"]), ("ProtocolRefused", 0))
                self.assertTrue(any(reason in failure for failure in receipt["failures"]), receipt)
                self.assertTrue(receipt["archive_post"]["exists"]);self.assertIn(receipt["archive_post"]["sha256"], foreign["quarantine"]["sha256"])
                self.assertEqual(len(receipt["archive_members_post"]), 1)
                self.assertEqual((call / "stdout.raw").read_bytes(), b"TEST_CODE_followup_stdout\n")
                self.assertEqual(len([json.loads(p.read_bytes()) for p in (session / "archive-entry").iterdir() if json.loads(p.read_bytes())["stage"] == "PSMIndexS"]), 1)
                self.assertTrue(any(b.startswith("ForeignOperation:" + call.name + ":") for b in foreign["blockers"]))
        for kind in ("output", "retained", "request_only"):
            with self.subTest(copy=kind):
                _, session, record, foreign, calls, controls = self.native_e5_result("follow_copy_" + kind)
                control = json.loads((session / "archive-copy-control.json").read_bytes());copy = Path(control["copy"])
                self.assertEqual(sha(copy), control["retained_sha256"]);self.assertIn(sha(copy), foreign["quarantine"]["sha256"])
                cc_path = session / "invocations" / control["cc_id"] / "receipt.json";cc = json.loads(cc_path.read_bytes())
                self.assertEqual(cc["source"], str(session / "vendor/cc/src/lib.rs"));self.assertEqual(cc["exit_code"], 0)
                self.assertEqual(sha(cc_path), control["cc_receipt_sha256"])
                bound = [c for c in record["invocations"] if c["invocation_id"] == control["cc_id"]]
                self.assertEqual(len(bound), 1);self.assertEqual(bound[0]["receipt_sha256"], sha(cc_path))
                self.assertFalse(any(c["path"] == str(copy) for c in record["consumed_sources"]))
                self.assertFalse(any(e["path"] == str(copy) and e["producers"] for e in record["extern_edges"]))
                self.assertIn("NativeOutputRole:" + (control["cc_id"] if kind == "output" else str(copy)), record["blockers"])
                if kind == "output":
                    self.assertIn("UnresolvedCargoArtifact:" + cc["source"], record["blockers"])
                    self.assertIn({"path": str(copy), "kind": "link"}, cc["declared_outputs"])
                    literal = str(session / "vendor" / owner.PROBE_LITERAL)
                    self.assertFalse(any(c["path"] == literal for c in record["consumed_sources"]))
                    self.assertFalse(any(control["cc_id"] in e["producers"] for e in record["extern_edges"]))
                    self.assertFalse(any(a["producer_invocation"] == control["cc_id"] for a in record["build_script_associations"]))
                if kind in ("retained", "request_only"):
                    self.assertNotIn(sha(copy), {sha(p) for p in (session / "foreign-native-invocations").glob("*/*.raw")})
                if kind == "retained":self.assertFalse(Path(calls[control["operation_id"]][1]["archive_post"]["path"]).exists())
                if kind == "request_only":
                    call = session / "foreign-native-invocations" / control["operation_id"]
                    self.assertTrue((call / "request.json").is_file());self.assertFalse((call / "receipt.json").exists())
                    request = json.loads((call / "request.json").read_bytes());archive = Path(os.fsdecode(bytes.fromhex(request["args_hex"][1])))
                    self.assertIn(str(archive), foreign["quarantine"]["paths"]);self.assertEqual(sha(archive), sha(copy))
                    self.assertEqual([o.get("protocol_state") for o in foreign["operations"] if o["operation_id"] == call.name], [None])


    def prepare_native_e6_ring_index(self, case):
        # E5 fixtures/globals stay exact. Only this fresh private copy gains s.
        inventory = self.prepare_native_e5("follow_normal")
        cargo = self.root / "fake-cargo"; body = cargo.read_text()
        self.assertEqual(body.count("CASE='follow_normal';argv="), 1)
        body = body.replace("CASE='follow_normal';argv=", "CASE=" + repr(case) + ";argv=")
        fragment = E5_ARCHIVE_CARGO
        needle = "            follow('RingRemainingCQ',raw)\n"
        self.assertEqual(fragment.count(needle), 1)
        fragment = fragment.replace(needle, "            if CASE not in ('ring_index_early','ring_index_first_partial_early'):follow('RingRemainingCQ',raw)\n")
        self.assertEqual(fragment.count("    role='CC'\n"), 1)
        fragment = fragment.replace("    role='CC'\n", E6_RING_INDEX_CARGO + "    role='CC'\n")
        self.assertEqual(body.count(E5_ARCHIVE_CARGO), 1); body = body.replace(E5_ARCHIVE_CARGO, fragment)
        if case.startswith("ring_index_copy_"):
            copy = E3_COPY_CARGO.replace("ar_copy_", "ring_index_copy_")
            old = "    p=next(p for p in paths if json.loads(p.read_text()).get('operation',{}).get('class')=='ArchiverFirstAppend'\n           and json.loads(p.read_text())['context']['manifest']==str(session/'vendor/ring'))"
            new = "    selected=[p for p in paths if json.loads(p.read_text()).get('operation',{}).get('class')=='ArchiverIndex' and json.loads(p.read_text())['context']['manifest']==str(session/'vendor/ring')];assert len(selected)==1;p=selected[0]"
            self.assertEqual(copy.count(old), 1); copy = copy.replace(old, new)
            needle = "emit({'reason':'build-finished','success':True})"; self.assertEqual(body.count(needle), 1)
            body = body.replace(needle, copy + "\n" + needle)
        cargo.write_text(body); inventory["cargo"]["sha256"] = sha(cargo)
        native = E5_NATIVE_AR
        self.assertEqual(native.count("case=='follow_partial'"), 2)
        native = native.replace("case=='follow_partial'", "(case=='follow_partial' or (case=='ring_index_first_partial_early' and name=='ring'))")
        self.assertEqual(native.count("case=='follow_tail_nonzero'"), 1)
        native = native.replace("case=='follow_tail_nonzero'", "case in ('follow_tail_nonzero','ring_index_tail_nonzero')")
        old = "'PSMIndexS' if args[0]=='s'"
        self.assertEqual(native.count(old), 1)
        native = native.replace(old, "('RingIndexS' if name=='ring' else 'PSMIndexS') if args[0]=='s'")
        needle = "sys.exit(7 if stage=='FirstD'"
        self.assertEqual(native.count(needle), 1)
        native = native.replace(needle, "if stage=='RingIndexS':\n    if case=='ring_index_member_post':(archive.parent/'25ac62e5b3c53843-curve25519.o').write_bytes(b'TEST_CODE_index_member_changed')\n    if case=='ring_index_signal':os.kill(os.getpid(),signal.SIGTERM)\n    if case=='ring_index_nonzero':sys.exit(11)\n" + needle)
        ar = self.root / "fake-native-ar"; ar.write_text("#!" + PYTHON + " -I\n" + native); ar.chmod(0o700)
        inventory["generators"]["AR"]["sha256"] = sha(ar)
        tool = self.tool.read_text()
        if case == "ring_index_capture":
            old = 'try:dest=open(call/(name+".raw"),"xb")'; self.assertEqual(tool.count(old), 1)
            tool = tool.replace(old, 'try:\n                if env.get("E1_CASE")=="ring_index_capture" and env.get("CARGO_PKG_NAME")=="ring" and argv[1:2]==["s"] and name=="stderr":raise OSError("TEST_CODE ring index capture")\n                dest=open(call/(name+".raw"),"xb")')
        if case == "ring_index_fd_return":
            old = 'receipt["tool_result"] = code; receipt["failures"].extend(faults)'; self.assertEqual(tool.count(old), 1)
            tool = tool.replace(old, old + '\n        if archiving and operation["class"]=="ArchiverIndex" and Path(context["manifest"]).name=="ring":os.close(fds[0])')
        self.tool.write_text(tool); inventory["owner_sha256"] = sha(self.tool)
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly", "profile": owner.BUNDLED_PROFILE, "inventory": inventory}))
        return inventory

    def native_e6_ring_index_result(self, case):
        inventory = self.prepare_native_e6_ring_index(case)
        run = subprocess.run([PYTHON, "-I", str(self.tool), "record"], env=dict(os.environ),
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=120)
        self.assertEqual(run.returncode, 2, run.stdout.decode(errors="replace") + run.stderr.decode(errors="replace"))
        record = self.record_result(run); session = Path(json.loads(run.stdout)["record_path"]).parent
        self.assertEqual(record["cargo_exit_code"], 0, (session / "cargo.stderr.raw").read_text(errors="replace"))
        foreign = json.loads((session / "foreign-native-record.json").read_bytes())
        self.assertEqual(record["foreign_native_record_sha256"], sha(session / "foreign-native-record.json"))
        self.assertEqual((foreign["stage"], foreign["native_producer_qualification"], foreign["artifact_selection"]),
                         ("StageCIncomplete", "not_issued", "not_observed")); self.assertEqual(record["selected_library"], [])
        calls = {p.parent.name: (p.parent, json.loads(p.read_bytes())) for p in (session / "foreign-native-invocations").glob("*/receipt.json")}
        controls = json.loads((session / "followups-control.json").read_bytes())
        index = json.loads((session / "ring-index-control.json").read_bytes())
        return inventory, session, record, foreign, calls, controls, index

    def test_native_e6_ring_index_full_chain_and_streamed_results(self):
        for case, code in (("ring_index_normal", 0), ("ring_index_nonzero", 11), ("ring_index_signal", -15)):
            with self.subTest(case=case):
                inventory, session, record, foreign, calls, controls, index = self.native_e6_ring_index_result(case)
                self.assertEqual(len(index), 1); control = index[0]; call, receipt = calls[control["operation_id"]]
                self.assertEqual(len(control["entry_new"]), 1)
                rows = [r for r in controls if r["name"] == "ring"]
                self.assertEqual([r["stage"] for r in rows], ["FirstD", "FirstFallbackCQ", "RingRemainingCQ", "RingIndexS"])
                self.assertEqual((receipt["protocol_state"], receipt["tool_result"], receipt["failures"]), ("Completed", code, []))
                self.assertEqual(control["status"], 143 if code == -15 else code)
                self.assertEqual(receipt["operation"]["class"], "ArchiverIndex"); self.assertEqual(control["args"], ["s", receipt["operation"]["archive"]])
                self.assertEqual(receipt["source_semantics"], {"index_success": code == 0, "archive_observed": True})
                self.assertEqual(receipt["archive_predecessor"], rows[-2]["operation_id"])
                self.assertEqual(receipt["archive_history_pre"], [{"operation_id": r["operation_id"], "request_sha256": sha(calls[r["operation_id"]][0] / "request.json"), "receipt_sha256": sha(calls[r["operation_id"]][0] / "receipt.json")} for r in rows[:-1]])
                self.assertEqual(receipt["archive_history_return"], receipt["archive_history_pre"])
                expected = [str(Path(receipt["context"]["out_dir"]) / n) for n in (*owner.FOREIGN_ARCHIVE_FIRST_MEMBERS["ring"], *owner.FOREIGN_ARCHIVE_REMAINING_MEMBERS)]
                self.assertEqual(receipt["operation"]["members"], expected); self.assertEqual(len(set(expected)), 29)
                pre, post = receipt["archive_operand_ledger_pre"], receipt["archive_operand_ledger_return"]
                self.assertEqual(pre, post); self.assertEqual(pre["archive_member_inventory"], "not_observed")
                self.assertEqual([p["path"] for p in pre["producers"]], expected)
                self.assertEqual(receipt["archive_member_producers_pre"], pre["producers"])
                self.assertEqual(receipt["archive_member_producers_return"], pre["producers"])
                previous = calls[rows[-2]["operation_id"]][1]["archive_post"]
                self.assertEqual({k:v for k,v in receipt["archive_pre"].items() if k != "snapshot"}, {k:v for k,v in previous.items() if k != "snapshot"})
                self.assertEqual(len(receipt["archive_members_pre"]), 29); self.assertEqual(len(receipt["archive_members_post"]), 29)
                for before, after, producer in zip(receipt["archive_members_pre"], receipt["archive_members_post"], pre["producers"]):
                    self.assertEqual({k:v for k,v in before.items() if k != "snapshot"}, {k:v for k,v in after.items() if k != "snapshot"})
                    cc_call, cc = calls[producer["operation_id"]]; self.assertEqual((cc["protocol_state"], cc["tool_result"], cc["failures"]), ("Completed", 0, []))
                    self.assertEqual(producer["request_sha256"], sha(cc_call / "request.json")); self.assertEqual(producer["receipt_sha256"], sha(cc_call / "receipt.json"))
                    self.assertEqual((before["path"], after["path"]), (producer["path"], producer["path"]))
                    self.assertEqual((before["sha256"], before["length"]), (cc["output_post"]["sha256"], cc["output_post"]["length"]))
                    for state in (before, after):self.assertEqual(sha(call / state["snapshot"]), state["sha256"])
                self.assertEqual(receipt["jobserver_identity"], receipt["jobserver_return"])
                hit = json.loads((session / "archive-entry" / control["entry_new"][0]).read_bytes())
                self.assertEqual(hit["argv"], control["args"]); self.assertEqual(hit["cwd"], receipt["context"]["manifest"]); self.assertTrue(hit["stdin_eof"])
                self.assertEqual(hit["fds"], [r["fd"] for r in receipt["jobserver_identity"]["endpoints"]]); self.assertEqual(hit["inodes"], [r["inode"] for r in receipt["jobserver_identity"]["endpoints"]])
                forwarded = [r for r in json.loads((session / "foreign-forwarded.json").read_bytes()) if r["name"] == "ring" and r["args"] == control["args"]]
                self.assertEqual(len(forwarded), 1); self.assertEqual(forwarded[0]["status"], control["status"])
                for stream in ("stdout", "stderr"):
                    self.assertEqual((call / (stream + ".raw")).read_bytes(), ("TEST_CODE_followup_" + stream + "\n").encode())
                    self.assertEqual(sha(call / (stream + ".raw")), receipt[stream + "_sha256"])
                    self.assertEqual(forwarded[0][stream + "_hex"], (call / (stream + ".raw")).read_bytes().hex())
                for state in (receipt["archive_pre"], receipt["archive_post"]):self.assertEqual(sha(call / state["snapshot"]), state["sha256"])
                self.assertTrue(receipt["archive_post"]["exists"]); self.assertGreater(receipt["archive_post"]["length"], 0)
                psm = [calls[r["operation_id"]][1] for r in controls if r["name"] == "psm"]
                self.assertEqual([r["tool_result"] for r in psm], [1, 0, 0]); self.assertEqual(len(psm[-1]["operation"]["members"]), 1)
                failures = [b for b in foreign["blockers"] if b.startswith("ForeignArchiveUnresolvedNonzero:")]
                self.assertEqual(failures, ["ForeignArchiveUnresolvedNonzero:" + call.name] if code else [])
                self.assertFalse(any(b.startswith("ForeignOperation:") for b in foreign["blockers"]), foreign["blockers"])

    def test_native_e6_ring_index_sticky_history_and_negative_ownership(self):
        before_child = {"ring_index_d": "ForeignArchiveTemplate", "ring_index_member": "ForeignArchiveTemplate",
            "ring_index_test_archive": "ForeignArchiveTemplate", "ring_index_archive_alias": "ForeignArchiveTemplate",
            "ring_index_member_pre": "NativeVersionChanged", "ring_index_early": "ForeignArchiveTemplate",
            "ring_index_first_partial_early": "ForeignArchivePredecessor", "ring_index_tail_nonzero": "ForeignArchivePredecessor",
            "ring_index_repeat": "ForeignArchivePredecessor", "ring_index_prior_raw_out": "ForeignArchiveReceiptBinding",
            "ring_index_prior_raw_cwd": "ForeignArchiveReceiptBinding", "ring_index_history_missing": "No such file",
            "ring_index_pin_ledger": "ForeignCompilePinChanged", "ring_index_ledger_order": "ForeignArchiveLedger"}
        for case, reason in before_child.items():
            with self.subTest(case=case):
                _, session, _, foreign, calls, controls, index = self.native_e6_ring_index_result(case)
                control = index[-1]; call, receipt = calls[control["operation_id"]]
                self.assertEqual((receipt["protocol_state"], receipt["tool_result"]), ("ProtocolRefused", None)); self.assertTrue(any(reason in r for r in receipt["failures"]), receipt)
                self.assertEqual(control["entry_new"], []); self.assertEqual(control["archive_before"], control["archive_after"])
                self.assertEqual(control["prior_before"], control["prior_after"])
                self.assertFalse((call / "stdout.raw").exists()); self.assertFalse((call / "stderr.raw").exists())
                self.assertIn("ForeignOperation:" + call.name + ":ForeignProtocolSticky", foreign["blockers"])
                self.assertEqual([calls[r["operation_id"]][1]["tool_result"] for r in controls if r["name"] == "psm"], [1, 0, 0])
                if case.startswith("ring_index_prior_raw_"):
                    poison = [r for r in controls if r["stage"] == "RefusedPrior"]; self.assertEqual(len(poison), 1)
                    pc, pr = calls[poison[0]["operation_id"]]
                    self.assertEqual((pr["protocol_state"], pr["tool_result"], pr["failures"]), ("ProtocolRefused", None, ["ForeignArchiveTemplate"]))
                    raw = json.loads((pc / "request.json").read_bytes()); field = "environment_hex" if case.endswith("_out") else "cwd_hex"
                    self.assertNotEqual(raw[field], pr[field]); self.assertEqual(pr["context"], receipt["context"])
                    self.assertFalse((pc / "stdout.raw").exists()); self.assertFalse((pc / "stderr.raw").exists())
                if case in ("ring_index_early", "ring_index_first_partial_early"):
                    ring = [r for r in controls if r["name"] == "ring"]
                    self.assertEqual([r["stage"] for r in ring], ["FirstD", "FirstFallbackCQ", "RingIndexS"])
                    first = calls[ring[0]["operation_id"]][1]
                    self.assertEqual(first["archive_post"]["exists"], case == "ring_index_first_partial_early")
        for case, reason in (("ring_index_member_post", "ForeignArchiveInputChanged"), ("ring_index_capture", "ForeignCaptureSticky"), ("ring_index_fd_return", "InvalidJobserverDescriptors")):
            with self.subTest(case=case):
                _, session, _, foreign, calls, _, index = self.native_e6_ring_index_result(case)
                control = index[0]; call, receipt = calls[control["operation_id"]]
                self.assertEqual(len(control["entry_new"]), 1); self.assertEqual((receipt["protocol_state"], receipt["tool_result"]), ("ProtocolRefused", 0))
                self.assertTrue(any(reason in r for r in receipt["failures"]), receipt); self.assertTrue(receipt["archive_post"]["exists"])
                self.assertEqual(sha(call / receipt["archive_post"]["snapshot"]), receipt["archive_post"]["sha256"])
                self.assertIn(receipt["archive_post"]["sha256"], foreign["quarantine"]["sha256"])
                self.assertEqual(len(receipt["archive_members_post"]), 29)
                self.assertIn("ForeignOperation:" + call.name + ":ForeignProtocolSticky", foreign["blockers"])
        for kind in ("source", "extern", "output", "retained", "request_only"):
            with self.subTest(copy=kind):
                _, session, record, foreign, calls, _, _ = self.native_e6_ring_index_result("ring_index_copy_" + kind)
                control = json.loads((session / "archive-copy-control.json").read_bytes()); copy = Path(control["copy"])
                self.assertEqual(copy.read_bytes().hex(), control["body_hex"]); self.assertEqual(sha(copy), control["retained_sha256"])
                self.assertIn(sha(copy), foreign["quarantine"]["sha256"])
                cc = session / "invocations" / control["cc_id"] / "receipt.json"
                self.assertEqual(sha(cc), control["cc_receipt_sha256"])
                bound = [r for r in record["invocations"] if r["invocation_id"] == control["cc_id"]]; self.assertEqual(len(bound), 1); self.assertEqual(bound[0]["receipt_sha256"], sha(cc))
                self.assertIn("NativeOutputRole:" + (control["cc_id"] if kind == "output" else str(copy)), record["blockers"])
                self.assertFalse(any(r["path"] == str(copy) for r in record["consumed_sources"])); self.assertFalse(any(r["path"] == str(copy) and r["producers"] for r in record["extern_edges"]))
                if kind in ("retained", "request_only"):
                    self.assertNotIn(sha(copy), {sha(p) for p in (session / "foreign-native-invocations").glob("*/*.raw")})
                    call = session / "foreign-native-invocations" / control["operation_id"]
                    if kind == "retained":
                        self.assertEqual((call / "request.json").read_bytes(), b'{TEST_CODE_bad_archive_request')
                        r = json.loads((call / "receipt.json").read_bytes()); self.assertEqual(r["archive_post"]["sha256"], sha(copy)); self.assertFalse(Path(r["archive_post"]["path"]).exists())
                    else:self.assertTrue((call / "request.json").is_file()); self.assertFalse((call / "receipt.json").exists())


    def prepare_native_e6(self, case="normal"):
        inventory = self.prepare_native_e4("normal")
        cargo = write(self.root / "fake-cargo", "#!" + PYTHON + " -I\n" + E6_CARGO.replace("__CASE__", repr(case)).replace("__E4_FACTS__", repr(E4_FACTS)))
        native = write(self.root / "fake-native-cc", "#!" + PYTHON + " -I\n" + E6_NATIVE)
        for path in (cargo, native): path.chmod(0o700)
        inventory["cargo"]["sha256"] = sha(cargo); inventory["generators"]["CC"]["sha256"] = sha(native)
        body = self.tool.read_text()
        replacements = {
            "capture": ('try:dest=open(call/(name+".raw"),"xb")', 'try:\n                if env.get("E6_CASE")=="capture" and "-c" in argv and name=="stderr":raise OSError("TEST_CODE E6 capture")\n                dest=open(call/(name+".raw"),"xb")'),
            "forward": ('written = os.write(1 if stream == "stdout" else 2, view)', 'if env.get("E6_CASE")=="forward" and flagging:raise OSError("TEST_CODE E6 forward")\n                            written = os.write(1 if stream == "stdout" else 2, view)'),
            "return_control": ('receipt["controls_return"] = foreign_controls(session, policy, owner, context)', 'receipt["controls_return"] = foreign_controls(session, policy, owner, context)\n            if env.get("E6_CASE")=="return_control" and flagging:receipt["controls_return"]["session_owner_sha256"]="0"*64'),
            "return_fd": ('receipt["tool_result"] = code; receipt["failures"].extend(faults)', 'receipt["tool_result"] = code; receipt["failures"].extend(faults)\n        if env.get("E6_CASE")=="return_fd" and flagging:os.close(fds[0])'),
            "post_output": ('receipt["output_post"] = native_snapshot(call, operation["output"], "output-post.raw", absent=True)', 'receipt["output_post"] = native_snapshot(call, operation["output"], "output-post.raw", absent=True)\n            if env.get("E6_CASE")=="post_output" and flagging:Path(operation["output"]).write_bytes(b"TEST_CODE post output drift")'),
            "return_output": ('if flagging:\n                native_state_check(call, receipt["output_post"], live=True)', 'if flagging:\n                if env.get("E6_CASE")=="return_output":Path(operation["output"]).write_bytes(b"TEST_CODE return output drift")\n                native_state_check(call, receipt["output_post"], live=True)'),
        }
        if case in replacements:
            needle, replacement = replacements[case]; self.assertEqual(body.count(needle), 1); body = body.replace(needle, replacement)
        self.tool.write_text(body); inventory["owner_sha256"] = sha(self.tool)
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly", "profile": owner.BUNDLED_PROFILE, "inventory": inventory}))
        return inventory

    def native_e6_result(self, case="normal"):
        inventory = self.prepare_native_e6(case)
        run = subprocess.run([PYTHON, "-I", str(self.tool), "record"], env=dict(os.environ), stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=180)
        self.assertEqual(run.returncode, 2, run.stdout.decode(errors="replace") + run.stderr.decode(errors="replace"))
        record = self.record_result(run); session = Path(json.loads(run.stdout)["record_path"]).parent
        self.assertEqual(record["cargo_exit_code"], 0, (session / "cargo.stderr.raw").read_text(errors="replace"))
        foreign = json.loads((session / "foreign-native-record.json").read_bytes())
        calls = {p.parent.name: (p.parent, json.loads(p.read_bytes())) for p in (session / "foreign-native-invocations").glob("*/receipt.json")}
        trace = json.loads((session / "e6-forwarded.json").read_bytes())
        self.assertEqual(record["foreign_native_record_sha256"], sha(session / "foreign-native-record.json"))
        # Deliberate malformed request copies are negative ownership cuts, never stage authority.
        expected_stage = "StageCIncomplete" if case == "role" else ("StageAIncomplete" if case in ("ambiguous", "duplicate_help", "duplicate_version", "help_zero") else "StageBIncomplete")
        self.assertEqual((foreign["stage"], foreign["native_producer_qualification"], record["selected_library"]), (expected_stage, "not_issued", []))
        self.assertEqual(foreign["artifact_selection"], "not_observed")
        self.assertTrue(all(r["operation"]["class"] != "CompilerObjectCompile" for _, r in calls.values() if isinstance(r.get("operation"), dict)))
        return inventory, session, record, foreign, calls, trace

    def test_native_e6_family_groups_and_three_flag_overwrites(self):
        for case in ("normal", "unsupported", "warning", "warning0"):
            with self.subTest(case=case):
                _, session, record, foreign, calls, trace = self.native_e6_result(case)
                self.assertFalse(any(b.startswith("ForeignOperation:") for b in foreign["blockers"]), foreign["blockers"])
                flags = sorted([(c, r) for c, r in calls.values() if r.get("operation", {}).get("class") == "CompilerFlagProbe"], key=lambda row: row[1]["operation"]["index"])
                self.assertEqual(len(flags), 3)
                previous = None; previous_output = None
                for index, (call, receipt) in enumerate(flags):
                    op = receipt["operation"]
                    self.assertEqual((op["flag"], op["predecessor"]), (owner.FOREIGN_FLAG_ORDER[index], previous))
                    self.assertEqual(receipt["protocol_state"], "Completed"); self.assertEqual(receipt["failures"], [])
                    self.assertEqual(receipt["controls_pre"], receipt["controls_post"]); self.assertEqual(receipt["controls_pre"], receipt["controls_return"])
                    self.assertEqual(receipt["jobserver_identity"], receipt["jobserver_return"])
                    self.assertEqual(receipt["family_pre"], receipt["family_return"])
                    self.assertEqual((receipt["family_pre"]["mapping_rule"], receipt["family_pre"]["execution_edge"]), ("RecordingOnlySourceOrderedCapture", "not_observed"))
                    self.assertEqual(receipt["input_pre"]["length"], 28); self.assertEqual(receipt["input_pre"]["sha256"], owner.FOREIGN_FLAG_LITERAL_DIGEST)
                    self.assertEqual({k:v for k,v in receipt["input_pre"].items() if k!="snapshot"}, {k:v for k,v in receipt["input_return"].items() if k!="snapshot"})
                    self.assertEqual(receipt["output_pre"]["exists"], index != 0)
                    if previous_output:self.assertEqual({k:v for k,v in receipt["output_pre"].items() if k!="snapshot"}, {k:v for k,v in previous_output.items() if k!="snapshot"})
                    previous = call.name; previous_output = receipt["output_return"]
                    self.assertEqual(receipt["source_semantics"]["supported"], case != "unsupported" or index == 0)
                    self.assertEqual(receipt["tool_result"], -15 if case=="unsupported" and index==2 else 0)
                    for stream in ("stdout", "stderr"):self.assertEqual(sha(call/(stream+".raw")), receipt[stream+"_sha256"])
                    for evidence in receipt["family_pre"]["base"] + receipt["family_pre"]["probe"]:
                        p, r = calls[evidence["operation_id"]]; self.assertEqual(sha(p/"receipt.json"), evidence["receipt_sha256"]); self.assertEqual(sha(p/"request.json"), evidence["request_sha256"])
                self.assertEqual(len({r["family_pre"]["base"][-1]["operation_id"] for _, r in flags}), 1)
                self.assertEqual(len({r["family_pre"]["probe"][-1]["operation_id"] for _, r in flags}), 3)
                entries = [json.loads(p.read_bytes()) for p in (session/"e6-entry").iterdir()]
                self.assertEqual(len(entries), len(calls)); self.assertTrue(all(e["stdin_eof"] and len(e["fds"])==2 for e in entries))
                self.assertEqual({row["status"] for row in trace if row["args"]==["-?"]}, {1})
                self.assertEqual({row["status"] for row in trace if row["args"]==["--version"]}, {0})
                if case in ("warning", "warning0"):
                    files = [(call, receipt) for call, receipt in calls.values() if receipt.get("operation", {}).get("class") == "CompilerFamilyFileProbe"]
                    first = [(call, receipt) for call, receipt in files if not receipt["operation"]["retry"]]
                    retries = [(call, receipt) for call, receipt in files if receipt["operation"]["retry"]]
                    self.assertEqual((len(first), len(retries)), (5, 5))
                    self.assertEqual({receipt["operation"]["predecessor"] for _, receipt in retries}, {call.name for call, _ in first})
                    for call, receipt in first:
                        self.assertEqual((receipt["protocol_state"], receipt["tool_result"], receipt["failures"]), ("Completed", 0 if case == "warning0" else 3, []))
                        self.assertTrue(receipt["source_semantics"]["warning_retry_requested"]); self.assertFalse(receipt["source_semantics"]["effective_stdout"])
                        self.assertEqual((call / "stdout.raw").read_bytes(), b"")
                        self.assertEqual((call / "stderr.raw").read_bytes(), b"-Wslash-u-filename TEST_CODE\n")
                        follow = [(other, result) for other, result in retries if result["operation"]["predecessor"] == call.name]
                        self.assertEqual(len(follow), 1); _, result = follow[0]
                        self.assertEqual((result["protocol_state"], result["tool_result"], result["failures"]), ("Completed", 0, []))
                        self.assertTrue(result["source_semantics"]["effective_stdout"]); self.assertFalse(result["source_semantics"]["warning_retry_requested"])
                        self.assertEqual(result["operation"]["source"], receipt["operation"]["source"])
                        self.assertEqual([os.fsdecode(bytes.fromhex(word)) for word in result["args_hex"]], ["-E", "--", receipt["operation"]["source"]])
                        self.assertEqual({key:value for key,value in result["input_pre"].items() if key != "snapshot"}, {key:value for key,value in receipt["input_post"].items() if key != "snapshot"})
                    self.assertFalse(any(blocker.startswith(("ForeignFamily", "ForeignFlag")) for blocker in foreign["blockers"]), foreign["blockers"])
                literal = str(session/"target/x86_64-apple-darwin/debug/build/zstd-sys-0123456789abcdef/out/flag_check.c")
                self.assertFalse(any(c["path"] == literal for c in record["consumed_sources"]))

    def test_native_e6_admission_sticky_and_prechild_lineage(self):
        controls = {"flag_order":"ForeignFlagArgv", "dedup":"ForeignSourceContext", "extra":"ForeignSourceContext", "flag_cwd":"ForeignEOnlyArgv",
                    "flag_locale":"ForeignCompileEnvironment", "flag_literal":"ForeignFlagLiteral", "output_pre":"ForeignFlagPredecessor",
                    "ambiguous":"ForeignFlagFamilyOrder", "missing_base":"ForeignFlagFamilyOrder", "duplicate_version":"ForeignFamilyUnique", "missing_receipt":"ForeignFamilyPending", "raw_out":"ForeignFamilyReceiptBinding", "role":"ForeignFamilyReceiptBinding",
                    "stream":"ForeignFamilyStream", "snapshot":"NativeSnapshotChanged", "duplicate_help":"ForeignFamilyUnique", "zig":"ForeignFamilyClang", "help_zero":"ForeignFamilyClang", "version_nonzero":"ForeignFamilyClang",
                    "post_input":"ForeignInputChanged", "post_output":"NativeVersionChanged", "capture":"ForeignCaptureSticky", "forward":"ForeignForward",
                    "return_control":"ForeignControlChanged", "return_fd":"InvalidJobserverDescriptors", "return_output":"NativeVersionChanged"}
        for case, marker in controls.items():
            with self.subTest(case=case):
                _, session, _, foreign, calls, trace = self.native_e6_result(case)
                refused = [(c,r) for c,r in calls.values() if r["protocol_state"] == "ProtocolRefused"]
                self.assertTrue(refused, calls)
                self.assertTrue(any(marker in f for _,r in refused for f in r["failures"]), [(r["args_hex"],r["failures"]) for _,r in refused])
                self.assertTrue(any("ForeignProtocolSticky" in b or "ForeignOperation:" in b for b in foreign["blockers"]))
                cut = json.loads((session/"e6-cut.json").read_bytes())
                self.assertEqual(cut["child_after"], cut["child_before"] + (1 if case in ("post_input","post_output","capture","forward","return_control","return_fd","return_output") else 0))
                target = [r for _,r in refused if r["args_hex"] == [os.fsencode(a).hex() for a in cut["argv"]]]
                self.assertTrue(target)
                if case not in ("post_input","post_output","capture","forward","return_control","return_fd","return_output"):
                    self.assertTrue(all(r["tool_result"] is None for r in target))
                if case in ("post_output", "forward", "return_control", "return_fd", "return_output"):
                    # Retain genuine child outcome even when a later protocol fault blocks use.
                    self.assertTrue(all(r["tool_result"] == 0 and r["source_semantics"]["supported"] for r in target))
                else:self.assertTrue(all("source_semantics" not in r for r in target))
                if "next_argv" in cut:
                    self.assertEqual(cut["next_child_before"], cut["next_child_after"])
                    following = [r for _, r in calls.values() if r["args_hex"] == [os.fsencode(a).hex() for a in cut["next_argv"]]]
                    self.assertEqual(len(following), 1); self.assertEqual((following[0]["protocol_state"], following[0]["tool_result"]), ("ProtocolRefused", None))
                    self.assertIn("ForeignFamilySticky", following[0]["failures"])

    def test_native_e6_flag_negative_namespace_and_retained_copies(self):
        for kind in ("source", "extern", "output"):
            for cut in ("current", "retained", "request_only"):
                with self.subTest(kind=kind, cut=cut):
                    _, session, record, foreign, calls, _ = self.native_e6_result("copy_"+kind+"_"+cut)
                    control = json.loads((session/"e6-copy.json").read_bytes()); copy = Path(control["copy"])
                    self.assertEqual(sha(copy), control["sha256"]); self.assertIn(sha(copy), foreign["quarantine"]["sha256"])
                    self.assertIn("NativeOutputRole:" + (control["cc_id"] if kind == "output" else str(copy)), record["blockers"])
                    self.assertFalse(any(c["path"]==str(copy) for c in record["consumed_sources"]))
                    self.assertFalse(any(e["path"]==str(copy) and e["producers"] for e in record["extern_edges"]))
                    cc = json.loads((session/"invocations"/control["cc_id"]/"receipt.json").read_bytes())
                    self.assertEqual(sha(session/"invocations"/control["cc_id"]/"receipt.json"), control["cc_receipt_sha256"])
                    literal = str(session/"vendor"/owner.PROBE_LITERAL)
                    if kind == "output":
                        self.assertIn("UnresolvedCargoArtifact:"+cc["source"], record["blockers"])
                        self.assertFalse(any(c["path"]==literal for c in record["consumed_sources"]))
                    else:self.assertTrue(any(c["path"]==literal for c in record["consumed_sources"]))
                    self.assertFalse(any(control["cc_id"] in e["producers"] for e in record["extern_edges"]))
                    self.assertFalse(any(a["producer_invocation"]==control["cc_id"] for a in record["build_script_associations"]))
                    if cut == "retained":self.assertNotIn(sha(copy), {sha(q) for q in (session/"foreign-native-invocations").glob("*/*.raw")})
                    if cut == "request_only":self.assertFalse((session/"foreign-native-invocations"/control["operation_id"]/"receipt.json").exists())
        for shape in ("none", "list"):
            with self.subTest(operation_shape=shape):
                _, session, record, foreign, calls, _ = self.native_e6_result("operation_" + shape)
                control = json.loads((session / "e6-operation-cut.json").read_bytes()); ident = control["operation_id"]
                call, receipt = calls[ident]; copy = Path(control["copy"]); snapshot = Path(control["snapshot"])
                self.assertEqual(receipt["operation"], None if shape == "none" else [])
                self.assertEqual((receipt["protocol_state"], receipt["tool_result"], receipt["failures"]), ("Completed", 0, []))
                self.assertEqual(sha(call / "receipt.json"), control["receipt_sha256"])
                self.assertEqual((sha(snapshot), sha(copy)), (control["sha256"], control["sha256"]))
                self.assertEqual(control["child_before"], control["child_after"])
                self.assertEqual(len(list((session / "e6-entry").iterdir())), control["child_after"])
                self.assertIn("ForeignOperation:" + ident + ":ForeignOperationFields", foreign["blockers"])
                self.assertIn(str(snapshot), foreign["quarantine"]["paths"]); self.assertIn(control["sha256"], foreign["quarantine"]["sha256"])
                item = next(row for row in foreign["operations"] if row["operation_id"] == ident)
                self.assertEqual(item["retained_raw_sha256"][snapshot.name], control["sha256"])
                self.assertIn("NativeOutputRole:" + str(copy), record["blockers"])
                self.assertFalse(any(row["path"] == str(copy) for row in record["consumed_sources"]))
                self.assertFalse(any(row["path"] == str(copy) and row["producers"] for row in record["extern_edges"]))
                self.assertEqual(sha(session / "invocations" / control["cc_id"] / "receipt.json"), control["cc_receipt_sha256"])
                self.assertTrue(any(row["path"] == str(session / "vendor" / owner.PROBE_LITERAL) for row in record["consumed_sources"]))
                self.assertFalse(any(control["cc_id"] in row["producers"] for row in record["extern_edges"]))
                self.assertEqual((record["selected_library"], foreign["native_producer_qualification"]), ([], "not_issued"))


    def prepare_native_e7_lz4(self, case="normal"):
        inventory = self.prepare_native_e4("normal")
        cargo = write(self.root / "fake-cargo", "#!" + PYTHON + " -I\n" + E7_CARGO.replace("__E7_CASE__", repr(case)).replace("__E4_FACTS__", repr(E4_FACTS)))
        native = write(self.root / "fake-native-cc", "#!" + PYTHON + " -I\n" + E7_NATIVE)
        for path in (cargo, native): path.chmod(0o700)
        inventory["cargo"]["sha256"] = sha(cargo); inventory["generators"]["CC"]["sha256"] = sha(native)
        body = self.tool.read_text()
        # Same closed fake-compiler map binding as E2; never a seal-success stub.
        original = "FOREIGN_MAP_RUSTC_SHA256 = " + json.dumps(owner.FOREIGN_MAP_RUSTC_SHA256)
        self.assertEqual(body.count(original), 1)
        body = body.replace(original, "FOREIGN_MAP_RUSTC_SHA256 = " + json.dumps(inventory["rustc"]["sha256"]))
        replacements = {
            "capture": ('try:dest=open(call/(name+".raw"),"xb")', 'try:\n                if env.get("E7_CASE")=="capture" and "-c" in argv and name=="stderr":raise OSError("TEST_CODE E7 capture")\n                dest=open(call/(name+".raw"),"xb")'),
            "forward": ('written = os.write(1 if stream == "stdout" else 2, view)', 'if env.get("E7_CASE")=="forward" and compiling:raise OSError("TEST_CODE E7 forward")\n                            written = os.write(1 if stream == "stdout" else 2, view)'),
            "return_control": ('receipt["controls_return"] = foreign_controls(session, policy, owner, context)', 'receipt["controls_return"] = foreign_controls(session, policy, owner, context)\n            if env.get("E7_CASE")=="return_control" and compiling:receipt["controls_return"]["session_owner_sha256"]="0"*64'),
            "return_fd": ('receipt["tool_result"] = code; receipt["failures"].extend(faults)', 'receipt["tool_result"] = code; receipt["failures"].extend(faults)\n        if env.get("E7_CASE")=="return_fd" and compiling:os.close(fds[0])'),
            "return_output": ('if compiling:\n                native_state_check(call, receipt["output_post"], live=True)', 'if compiling:\n                if env.get("E7_CASE")=="return_output":Path(operation["output"]).write_bytes(b"TEST_CODE E7 return object drift")\n                native_state_check(call, receipt["output_post"], live=True)'),
        }
        if case in replacements:
            needle, replacement = replacements[case]; self.assertEqual(body.count(needle), 1); body = body.replace(needle, replacement)
        if case == "lock_fd":
            needle = 'receipt["serialization_post"] = foreign_lz4_lock_state(session, *lz4_lock)'; self.assertEqual(body.count(needle), 1)
            body = body.replace(needle, 'if env.get("E7_CASE")=="lock_fd":os.close(lz4_lock[0])\n                ' + needle)
        if case == "crash":
            needle = '    atomic_json(call / "request.json", request)'; self.assertEqual(body.count(needle), 1)
            body = body.replace(needle, needle + '\n    if env.get("E7_CASE")=="crash" and args[-1:]==["liblz4/lib/lz4.c"]:os._exit(19)\n')
        if case == "parallel":
            # Filled by the fixed session-owner serialization seam; no fake receipt.
            needle = '    atomic_json(call / "request.json", request)'; self.assertEqual(body.count(needle), 1)
            pause = '\n    if env.get("E7_CASE")=="parallel" and args[-1:]==["liblz4/lib/lz4.c"]:\n        import time\n        (session/"e7-pending.json").write_text(json.dumps({"operation_id":call.name}))\n        deadline=time.monotonic()+15\n        while not (session/"e7-release").exists():\n            require(time.monotonic()<deadline,"TEST_CODE E7 pending release");time.sleep(0.01)\n'
            body = body.replace(needle, needle + pause)
            needle = '        fcntl.flock(fd, fcntl.LOCK_EX)'; self.assertEqual(body.count(needle), 1)
            body = body.replace(needle, '        if env.get("E7_CASE")=="parallel" and args[-1:]==["liblz4/lib/lz4frame.c"]:(session/"e7-waiting").write_bytes(b"TEST_CODE lock wait")\n' + needle)
        self.tool.write_text(body); inventory["owner_sha256"] = sha(self.tool)
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly", "profile": owner.BUNDLED_PROFILE, "inventory": inventory}))
        return inventory

    def native_e7_lz4_result(self, case="normal"):
        inventory = self.prepare_native_e7_lz4(case)
        run = subprocess.run([PYTHON, "-I", str(self.tool), "record"], env=dict(os.environ), stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=240)
        self.assertEqual(run.returncode, 2, run.stdout.decode(errors="replace") + run.stderr.decode(errors="replace"))
        record = self.record_result(run); session = Path(json.loads(run.stdout)["record_path"]).parent
        self.assertEqual(record["cargo_exit_code"], 0, (session / "cargo.stderr.raw").read_text(errors="replace"))
        foreign = json.loads((session / "foreign-native-record.json").read_bytes())
        calls = {p.parent.name: (p.parent, json.loads(p.read_bytes())) for p in (session / "foreign-native-invocations").glob("*/receipt.json")}
        trace = json.loads((session / "e7-forwarded.json").read_bytes())
        self.assertEqual(record["foreign_native_record_sha256"], sha(session / "foreign-native-record.json"))
        stage = "StageCIncomplete" if case == "prior_role" else "StageBIncomplete"
        self.assertEqual((foreign["stage"], foreign["native_producer_qualification"], record["selected_library"]), (stage, "not_issued", []))
        self.assertEqual(foreign["artifact_selection"], "not_observed")
        return inventory, session, record, foreign, calls, trace

    def native_e7_lz4_private_negative_result(self, case):
        self.assertIn(case, ("header_pre", "header_post", "helper_pre", "helper_post"))
        from unittest import mock
        inventory = self.prepare_native_e7_lz4(case); pending_root = self.root / ".replay-build-records"
        before = set(pending_root.glob("pending-*"))
        run = subprocess.run([PYTHON, "-I", str(self.tool), "record"], env=dict(os.environ), stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=240)
        self.assertEqual(run.returncode, 2, run.stderr.decode(errors="replace")); self.assertEqual(run.stdout, b"")
        self.assertEqual(json.loads(run.stderr), {"schema": owner.SCHEMA, "state": "RecordingOnly", "reason": "Refused", "detail": "InventoryMismatch"})
        pending = set(pending_root.glob("pending-*")) - before; self.assertEqual(len(pending), 1); session = pending.pop()
        self.assertFalse((session / "record.json").exists()); self.assertFalse((session / "foreign-native-record.json").exists())
        self.assertTrue((session / "e7-forwarded.json").is_file(), (session / "cargo.stderr.raw").read_text(errors="replace"))
        calls = {p.parent.name: (p.parent, json.loads(p.read_bytes())) for p in (session / "foreign-native-invocations").glob("*/receipt.json")}
        with mock.patch.object(owner, "POLICY", self.policy):
            negative = owner.foreign_evidence_namespace(session)
        return inventory, session, calls, negative

    def test_native_e7_lz4_four_compile_only_objects_and_serialized_peers(self):
        for case in ("normal", "parallel"):
            with self.subTest(case=case):
                _, session, record, foreign, calls, trace = self.native_e7_lz4_result(case)
                compiles = [(call, receipt) for call, receipt in calls.values() if isinstance(receipt.get("operation"), dict) and receipt["operation"].get("scope") == "CompileOnly"]
                self.assertEqual(len(compiles), 4)
                self.assertEqual({r["operation"]["raw_source"] for _, r in compiles}, set(owner.FOREIGN_LZ4_ORDINARY_SOURCES))
                for call, receipt in compiles:
                    op = receipt["operation"]
                    self.assertEqual((receipt["protocol_state"], receipt["tool_result"], receipt["failures"]), ("Completed", 0, []))
                    self.assertEqual(op["object_derivation"], {"dirname": "liblz4/lib", "extension": "c", "prefix": "efce31824dbf3730"})
                    self.assertEqual(op["output"], str(Path(receipt["context"]["out_dir"]) / ("efce31824dbf3730-" + Path(op["raw_source"]).with_suffix(".o").name)))
                    self.assertEqual(receipt["source_semantics"], {"compiler_success": True, "object_observed": True})
                    self.assertEqual(receipt["controls_pre"], receipt["controls_post"]); self.assertEqual(receipt["controls_pre"], receipt["controls_return"])
                    self.assertEqual(receipt["compile_pins_pre"], receipt["compile_pins_post"]); self.assertEqual(receipt["compile_pins_pre"], receipt["compile_pins_return"])
                    self.assertEqual(receipt["jobserver_identity"], receipt["jobserver_return"])
                    self.assertEqual(receipt["serialization_pre"], receipt["serialization_post"]); self.assertEqual(receipt["serialization_pre"], receipt["serialization_return"])
                    self.assertEqual((receipt["serialization_pre"]["scope"], receipt["serialization_pre"]["path"]), ("SerializationOnly", str(session / "owner.json")))
                    self.assertEqual(receipt["family_pre"], receipt["family_return"])
                    self.assertEqual((receipt["family_pre"]["mapping_rule"], receipt["family_pre"]["execution_edge"]), ("RecordingOnlySourceOrderedCapture", "not_observed"))
                    self.assertEqual(receipt["family_pre"]["base"], receipt["family_pre"]["probe"])
                    for kind in ("input", "output"):
                        self.assertEqual({k:v for k,v in receipt[kind + "_post"].items() if k != "snapshot"}, {k:v for k,v in receipt[kind + "_return"].items() if k != "snapshot"})
                        self.assertEqual(sha(call / receipt[kind + "_return"]["snapshot"]), receipt[kind + "_return"]["sha256"])
                    self.assertFalse(receipt["output_pre"]["exists"]); self.assertGreater(receipt["output_post"]["length"], 0)
                    for stream in ("stdout", "stderr"):
                        self.assertEqual(sha(call / (stream + ".raw")), receipt[stream + "_sha256"])
                        row = next(row for row in trace if row["args"] == [os.fsdecode(bytes.fromhex(v)) for v in receipt["args_hex"]])
                        self.assertEqual(row[stream + "_hex"], (call / (stream + ".raw")).read_bytes().hex())
                    self.assertFalse(any(row["path"] == op["source"] for row in record["consumed_sources"]))
                    self.assertFalse(any(row["path"] == op["output"] and row["producers"] for row in record["extern_edges"]))
                    self.assertFalse(any(blocker.startswith("ForeignOperation:" + call.name + ":") for blocker in foreign["blockers"]), foreign["blockers"])
                denied = json.loads((session / "e7-zstd-cut.json").read_bytes()); self.assertEqual(denied["child_before"], denied["child_after"])
                selected = [(call, receipt) for call, receipt in calls.values() if [os.fsdecode(bytes.fromhex(v)) for v in receipt["args_hex"]] == denied["argv"]]
                self.assertEqual(len(selected), 1); _, receipt = selected[0]
                self.assertEqual((receipt["protocol_state"], receipt["tool_result"]), ("ProtocolRefused", None)); self.assertIn("ForeignEOnlyArgv", receipt["failures"])
                entries = [json.loads(path.read_bytes()) for path in (session / "e6-entry").iterdir()]
                self.assertEqual(len(entries), len([r for _, r in calls.values() if r["tool_result"] is not None]))
                self.assertTrue(all(e["stdin_eof"] and len(e["fds"]) == 2 and e["regular_fds"] == [] for e in entries))
                if case == "parallel":
                    queued = json.loads((session / "e7-serialization.json").read_bytes()); self.assertEqual(queued["queued_after"], queued["queued_before"] + 2)
                if case == "normal":
                    fd = os.open(session / "owner.json", os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
                    try:
                        owner.fcntl.flock(fd, owner.fcntl.LOCK_EX)
                        specification = importlib.util.spec_from_file_location("record_owner_e7_private", self.tool)
                        private_owner = importlib.util.module_from_spec(specification); specification.loader.exec_module(private_owner)
                        held = private_owner.foreign_seal(session, json.loads(self.policy.read_bytes()))
                        self.assertTrue(any("ForeignCompileInFlight" in reason for reason in held["blockers"]), held["blockers"])
                    finally: os.close(fd)

    def test_native_e7_lz4_exact_admission_and_post_faults_stay_sticky(self):
        controls = {
            "argv": "ForeignCompileArgv", "source": "ForeignCompileSource", "locale": "ForeignCompileEnvironment",
            "features": "ForeignFeaturesLinks", "jobs": "ForeignCompileConfiguration", "invalid_fd": "InvalidJobserverDescriptors", "output_alias": "ForeignCompileOutputAlias", "output_pre": "ForeignCompileOwnership", "missing_object": "ForeignCompileObjectMissing",
            "nonzero": "ForeignFamilySticky", "signed": "ForeignFamilySticky", "capture": "ForeignCaptureSticky", "forward": "ForeignForward:stdout:",
            "return_control": "ForeignControlChanged", "return_fd": "InvalidJobserverDescriptors", "return_output": "NativeVersionChanged", "lock_fd": "Bad file descriptor", "crash": "ForeignFamilyPending",
            "prior_raw_out": "ForeignFamilyReceiptBinding", "prior_role": "ForeignFamilyReceiptBinding", "prior_stream": "ForeignFamilyStream",
            "prior_receipt": "ForeignFamilyPending", "prior_snapshot": "NativeSnapshotChanged", "prior_output": "NativeVersionChanged",
        }
        child_cases = {"missing_object", "nonzero", "signed", "capture", "forward", "return_control", "return_fd", "return_output", "lock_fd"}
        for case, marker in controls.items():
            with self.subTest(case=case):
                _, session, _, foreign, calls, _ = self.native_e7_lz4_result(case)
                cut = json.loads((session / "e7-cut.json").read_bytes())
                if case == "crash":
                    self.assertEqual(cut["status"], 19); self.assertNotIn(cut["operation_id"], calls)
                    self.assertTrue((session / "foreign-native-invocations" / cut["operation_id"] / "request.json").is_file())
                    self.assertEqual(cut["child_before"], cut["child_after"]); self.assertEqual(cut["next_child_before"], cut["next_child_after"])
                    _, next_receipt = calls[cut["next_operation_id"]]
                    self.assertEqual((next_receipt["protocol_state"], next_receipt["tool_result"]), ("ProtocolRefused", None))
                    self.assertIn(marker, next_receipt["failures"]); continue
                call, receipt = calls[cut["operation_id"]]
                self.assertEqual(cut["child_after"], cut["child_before"] + (1 if case in child_cases else 0))
                if case in ("nonzero", "signed"):
                    self.assertEqual((receipt["protocol_state"], receipt["tool_result"], receipt["failures"]), ("Completed", 7 if case == "nonzero" else -15, []))
                    self.assertFalse(receipt["source_semantics"]["compiler_success"])
                    next_call, next_receipt = calls[cut["next_operation_id"]]
                    self.assertIn(marker, next_receipt["failures"])
                else:
                    self.assertEqual(receipt["protocol_state"], "ProtocolRefused")
                    self.assertTrue(any(marker in reason for reason in receipt["failures"]), receipt["failures"])
                    self.assertEqual(receipt["tool_result"], 0 if case in child_cases else None)
                if "next_operation_id" in cut:
                    _, next_receipt = calls[cut["next_operation_id"]]
                    self.assertEqual(cut["next_child_before"], cut["next_child_after"])
                    self.assertEqual((next_receipt["protocol_state"], next_receipt["tool_result"]), ("ProtocolRefused", None))
                self.assertTrue(any(b.startswith("ForeignProtocolSticky:") or b.startswith("ForeignOperation:") for b in foreign["blockers"]), foreign["blockers"])

    def test_native_e7_lz4_negative_owned_objects_and_private_header_cuts(self):
        for kind in ("source", "extern", "output"):
            for cut in ("current", "retained", "request_only", "stream"):
                with self.subTest(kind=kind, cut=cut):
                    _, session, record, foreign, calls, _ = self.native_e7_lz4_result("copy_" + kind + "_" + cut)
                    control = json.loads((session / "e7-copy.json").read_bytes()); copy = Path(control["copy"])
                    self.assertEqual(sha(copy), control["sha256"]); self.assertIn(control["sha256"], foreign["quarantine"]["sha256"])
                    self.assertIn("NativeOutputRole:" + (control["cc_id"] if kind == "output" else str(copy)), record["blockers"])
                    self.assertFalse(any(c["path"] == str(copy) for c in record["consumed_sources"]))
                    self.assertFalse(any(e["path"] == str(copy) and e["producers"] for e in record["extern_edges"]))
                    self.assertFalse(any(control["cc_id"] in e["producers"] for e in record["extern_edges"]))
                    self.assertFalse(any(a["producer_invocation"] == control["cc_id"] for a in record["build_script_associations"]))
                    self.assertEqual(sha(session / "invocations" / control["cc_id"] / "receipt.json"), control["cc_receipt_sha256"])
                    if cut == "retained": self.assertNotIn(control["sha256"], {sha(p) for p in (session / "foreign-native-invocations").glob("*/*.raw")})
                    if cut == "request_only": self.assertFalse((session / "foreign-native-invocations" / control["operation_id"] / "receipt.json").exists())
                    if kind == "output":
                        cc = json.loads((session / "invocations" / control["cc_id"] / "receipt.json").read_bytes())
                        self.assertIn("UnresolvedCargoArtifact:" + cc["source"], record["blockers"])
        for case in ("header_pre", "header_post", "helper_pre", "helper_post"):
            with self.subTest(case=case):
                _, session, calls, negative = self.native_e7_lz4_private_negative_result(case)
                paths, probes, hashes, blockers = negative; self.assertIsInstance(negative, tuple)
                member = "cc/src/tool.rs" if case.startswith("helper_") else "lz4-sys/liblz4/lib/lz4.h"
                header = session / "vendor" / member
                if case.startswith("helper_"):
                    self.assertNotIn(str(header), paths); self.assertNotIn(sha(header), hashes)
                else:
                    self.assertIn(str(header), paths); self.assertIn(sha(header), hashes)
                    self.assertIn(owner.FOREIGN_E_ONLY_SOURCE_PINS[member], hashes)
                cut = json.loads((session / "e7-cut.json").read_bytes()); _, receipt = calls[cut["operation_id"]]
                self.assertEqual(cut["child_after"], cut["child_before"] + (1 if case.endswith("_post") else 0))
                self.assertEqual((receipt["protocol_state"], receipt["tool_result"]), ("ProtocolRefused", 0 if case.endswith("_post") else None))
                self.assertTrue(any("ForeignEOnlySourcePin" in reason or "ForeignCompileSourcePin" in reason for reason in receipt["failures"]), receipt["failures"])
                self.assertEqual(cut["next_child_before"], cut["next_child_after"])
                _, following = calls[cut["next_operation_id"]]; self.assertEqual((following["protocol_state"], following["tool_result"]), ("ProtocolRefused", None))

    def prepare_native_e8_ring_test(self, case):
        inventory = self.prepare_native_e6_ring_index("ring_index_normal")
        vendor = self.root / "vendor-origin"
        fixture = TOOL.parent.parent / "fixture-source/ring/crypto/constant_time_test.c"
        self.assertEqual(sha(fixture), owner.RING_AUX_SOURCE_SHA256)
        write(vendor / "ring/crypto/constant_time_test.c", fixture.read_text())
        inventory["vendor"] = snapshot(vendor, ["dep", "libsqlite3-sys", "cc", "diesel", "rusqlite", "ring", "psm"])
        cargo = self.root / "fake-cargo"; body = cargo.read_text()
        old = "CASE='ring_index_normal';argv="; self.assertEqual(body.count(old), 1)
        body = body.replace(old, "CASE=" + repr(case) + ";argv=")
        old = E6_RING_INDEX_CARGO + "    role='CC'\n"; self.assertEqual(body.count(old), 1)
        body = body.replace(old, E6_RING_INDEX_CARGO + E8_AUX_CARGO + "    role='CC'\n")
        old = "emit({'reason':'build-finished','success':True})"; self.assertEqual(body.count(old), 1)
        body = body.replace(old, E8_AUX_COPY + "\n" + old); cargo.write_text(body)
        native = self.root / "fake-native-cc"; body = native.read_text()
        old = "if '-c' in args:"; self.assertEqual(body.count(old), 1)
        native.write_text(body.replace(old, E8_AUX_CC + "\n" + old))
        ar = self.root / "fake-native-ar"; body = ar.read_text()
        old = "name=env['CARGO_PKG_NAME']\n"; self.assertEqual(body.count(old), 1)
        ar.write_text(body.replace(old, old + E8_AUX_AR))
        body = self.tool.read_text()
        if case == "aux_capture":
            old = 'try:dest=open(call/(name+".raw"),"xb")'; self.assertEqual(body.count(old), 1)
            body = body.replace(old, 'try:\n                if env.get("E1_CASE")=="aux_capture" and "-c" in argv and Path(argv[-1]).name=="constant_time_test.c" and name=="stderr":raise OSError("TEST_CODE Aux capture")\n                dest=open(call/(name+".raw"),"xb")')
        if case == "aux_return_fd":
            old = 'receipt["tool_result"] = code; receipt["failures"].extend(faults)'; self.assertEqual(body.count(old), 1)
            body = body.replace(old, old + '\n        if env.get("E1_CASE")=="aux_return_fd" and compiling and auxiliary:os.close(fds[0])')
        self.tool.write_text(body)
        inventory["cargo"]["sha256"] = sha(cargo); inventory["owner_sha256"] = sha(self.tool)
        inventory["generators"]["CC"]["sha256"] = sha(native); inventory["generators"]["AR"]["sha256"] = sha(ar)
        self.policy.write_text(json.dumps({"schema": owner.SCHEMA, "mode": "RecordingOnly", "profile": owner.BUNDLED_PROFILE, "inventory": inventory}))
        return inventory

    def native_e8_ring_test_result(self, case):
        inventory = self.prepare_native_e8_ring_test(case)
        run = subprocess.run([PYTHON, "-I", str(self.tool), "record"], env=dict(os.environ), stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=240)
        self.assertEqual(run.returncode, 2, run.stdout.decode(errors="replace") + run.stderr.decode(errors="replace"))
        record = self.record_result(run); session = Path(json.loads(run.stdout)["record_path"]).parent
        self.assertEqual(record["cargo_exit_code"], 0, (session / "cargo.stderr.raw").read_text(errors="replace"))
        foreign = json.loads((session / "foreign-native-record.json").read_bytes())
        self.assertEqual(record["foreign_native_record_sha256"], sha(session / "foreign-native-record.json"))
        self.assertEqual((foreign["stage"], foreign["native_producer_qualification"], foreign["artifact_selection"], record["selected_library"]), ("StageCIncomplete", "not_issued", "not_observed", []))
        calls = {p.parent.name: (p.parent, json.loads(p.read_bytes())) for p in (session / "foreign-native-invocations").glob("*/receipt.json")}
        control = json.loads((session / "e8-control.json").read_bytes())
        return inventory, session, record, foreign, calls, control

    def native_e8_ring_test_private_result(self, case):
        from unittest import mock
        self.assertIn(case, ("aux_source_pre", "aux_source_post"))
        inventory = self.prepare_native_e8_ring_test(case); root = self.root / ".replay-build-records"; before = set(root.glob("pending-*"))
        run = subprocess.run([PYTHON, "-I", str(self.tool), "record"], env=dict(os.environ), stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=240)
        self.assertEqual(run.returncode, 2, run.stderr.decode(errors="replace")); self.assertEqual(run.stdout, b"")
        self.assertEqual(json.loads(run.stderr), {"schema": owner.SCHEMA, "state": "RecordingOnly", "reason": "Refused", "detail": "InventoryMismatch"})
        pending = set(root.glob("pending-*")) - before; self.assertEqual(len(pending), 1); session = pending.pop()
        self.assertFalse((session / "record.json").exists()); self.assertFalse((session / "foreign-native-record.json").exists())
        self.assertTrue((session / "e8-control.json").is_file(), (session / "cargo.stderr.raw").read_text(errors="replace"))
        calls = {p.parent.name: (p.parent, json.loads(p.read_bytes())) for p in (session / "foreign-native-invocations").glob("*/receipt.json")}
        with mock.patch.object(owner, "POLICY", self.policy): negative = owner.foreign_evidence_namespace(session)
        return session, calls, json.loads((session / "e8-control.json").read_bytes()), negative

    def test_native_e8_two_ring_builds_c1_and_independent_test_archive_branches(self):
        for case in ("aux_fallback", "aux_partial_fallback", "aux_deterministic"):
            with self.subTest(case=case):
                _, session, record, foreign, calls, control = self.native_e8_ring_test_result(case)
                self.assertFalse(any(b.startswith("ForeignOperation:") for b in foreign["blockers"]), foreign["blockers"])
                main = [r for _, r in calls.values() if r.get("operation", {}).get("class") == "CompilerObjectCompile" and r["context"]["manifest"].endswith("/ring") and "ring_build" not in r["operation"]]
                self.assertEqual(len(main), 29); f0 = main[0]["family_pre"]
                self.assertTrue(all(r["family_pre"] == r["family_return"] == f0 for r in main))
                rows = control["rows"]; self.assertEqual([r["status"] for r in rows[:3]], [0, 1, 0])
                c = next(r for r in rows if r["stage"] == "C1"); _, cr = calls[c["operation_id"]]
                self.assertEqual((cr["protocol_state"], cr["tool_result"]), ("Completed", 0)); f1 = cr["family_pre"]
                self.assertEqual(f1, cr["family_return"]); self.assertNotEqual(f0, f1)
                self.assertTrue({x["operation_id"] for v in f0["evidence"].values() for x in v}.isdisjoint({x["operation_id"] for v in f1["evidence"].values() for x in v}))
                self.assertEqual(cr["output_post"]["path"], control["c1_object"]); self.assertEqual(cr["output_return"], cr["output_post"])
                self.assertEqual(cr["input_return"], cr["input_post"]); self.assertEqual(cr["main_archive_checkpoint_pre"], cr["main_archive_checkpoint_return"])
                self.assertEqual(cr["main_archive_checkpoint_pre"]["operation_id"], control["main_index_id"])
                self.assertEqual(len(cr["main_archive_checkpoint_pre"]["ledger"]["producers"]), 29)
                ar = [r for r in rows if r["stage"].startswith("Aux") and r["stage"] not in ("AuxE", "AuxH", "AuxV")]
                self.assertEqual([r["stage"] for r in ar], ["AuxFirstD", "AuxIndexSD"] if case == "aux_deterministic" else ["AuxFirstD", "AuxFallbackCQ", "AuxIndexS"])
                self.assertEqual([r["status"] for r in ar], [0, 0] if case == "aux_deterministic" else [7 if case == "aux_partial_fallback" else 1, 0, 0])
                for row in ar:
                    call, receipt = calls[row["operation_id"]]
                    self.assertEqual((receipt["protocol_state"], receipt["family_pre"], receipt["family_return"]), ("Completed", f1, f1))
                    self.assertEqual(receipt["operation"]["archive"], control["aux_archive"])
                    self.assertEqual(receipt["operation"]["members"], [control["c1_object"]])
                    self.assertEqual(receipt["archive_member_producers_pre"][0]["operation_id"], c["operation_id"])
                    self.assertEqual(receipt["archive_member_producers_pre"], receipt["archive_member_producers_return"])
                    self.assertEqual(receipt["main_archive_checkpoint_pre"], cr["main_archive_checkpoint_pre"])
                    self.assertEqual(receipt["controls_pre"], receipt["controls_post"]); self.assertEqual(receipt["controls_pre"], receipt["controls_return"])
                    self.assertEqual(receipt["jobserver_identity"], receipt["jobserver_return"])
                    phase_env = {os.fsdecode(bytes.fromhex(k)): os.fsdecode(bytes.fromhex(v)) for k, v in receipt["environment_hex"].items()}
                    if row["stage"] == "AuxIndexSD": self.assertNotIn("ZERO_AR_DATE", phase_env)
                    else: self.assertEqual(phase_env["ZERO_AR_DATE"], "1")
                    for stream in ("stdout", "stderr"): self.assertEqual(sha(call / (stream + ".raw")), receipt[stream + "_sha256"])
                self.assertEqual(len(calls[ar[-1]["operation_id"]][1]["archive_operand_ledger_return"]["producers"]), 1)
                if case == "aux_partial_fallback":
                    d = calls[ar[0]["operation_id"]][1]; fallback = calls[ar[1]["operation_id"]][1]
                    self.assertTrue(d["archive_post"]["exists"]); self.assertEqual(d["archive_post"]["sha256"], fallback["archive_pre"]["sha256"])
                self.assertEqual(sha(Path(control["main_archive"])), control["main_sha256"])
                psm = [r for _, r in calls.values() if r.get("role") == "ar" and r.get("context", {}).get("manifest", "").endswith("/psm")]
                self.assertEqual(sorted(r["tool_result"] for r in psm), [0, 0, 1])

    def test_native_e8_ring_c1_and_aux_history_faults_block_next_native_child(self):
        cases = {"aux_deterministic_wrong_zero": "ForeignArchiveEnvironment", "aux_early_sd": "ForeignArchivePredecessor",
                 "aux_missing_family": "ForeignRingFamilyReference", "aux_extra_family": "ForeignFamilyUnique",
                 "aux_missing_receipt": "ForeignFamilyPending", "aux_stream_drift": "ForeignFamilyStream",
                 "aux_main_family": "ForeignRingMainFamily", "aux_main_index_missing": "ForeignArchiveReceiptBinding",
                 "aux_main_raw_out": "ForeignFamilyReceiptBinding", "aux_wrong_object": "ForeignCompileArgv",
                 "aux_wrong_flags": "ForeignCompileArgv", "aux_duplicate_c1": "ForeignCompileOwnership",
                 "aux_capture": "ForeignCaptureSticky", "aux_return_fd": "InvalidJobserverDescriptors",
                 "aux_early_index": "ForeignArchivePredecessor", "aux_duplicate_member": "ForeignArchiveTemplate",
                 "aux_archive_drift": "NativeVersionChanged", "aux_prior_raw_out": "ForeignFamilyReceiptBinding",
                 "aux_prior_receiptless": "ForeignArchiveReceiptBinding", "aux_existing_archive": "ForeignArchiveInitial"}
        for case, marker in cases.items():
            with self.subTest(case=case):
                _, session, _, foreign, calls, control = self.native_e8_ring_test_result(case)
                refused = [(r, calls[r["operation_id"]][1]) for r in control["rows"] if r["operation_id"] in calls and calls[r["operation_id"]][1]["protocol_state"] == "ProtocolRefused"]
                self.assertTrue(refused, control); self.assertTrue(any(marker in reason for _, r in refused for reason in r["failures"]), refused)
                row, receipt = refused[-1]; self.assertEqual(receipt["tool_result"], None)
                self.assertEqual(row["child_after"], row["child_before"])
                self.assertTrue(any("ForeignOperation:" + row["operation_id"] + ":ForeignProtocolSticky" == b for b in foreign["blockers"]), foreign["blockers"])
                self.assertEqual(sha(Path(control["main_archive"])), control["main_sha256"])
                if case in ("aux_deterministic_wrong_zero", "aux_early_sd"):
                    first = next(r for r in control["rows"] if r["stage"] == "AuxFirstD"); call, prior = calls[first["operation_id"]]
                    self.assertEqual((prior["protocol_state"], prior["tool_result"]), ("Completed", 0 if case == "aux_deterministic_wrong_zero" else 1))
                    self.assertEqual(first["child_after"], first["child_before"] + 1); self.assertEqual(sha(call / "receipt.json"), first["receipt_sha256"])
                    denied = next(r for r in control["rows"] if r["stage"] == ("AuxIndexSD" if case == "aux_deterministic_wrong_zero" else "EarlyIndexSD"))
                    _, refused_sd = calls[denied["operation_id"]]
                    self.assertEqual((refused_sd["protocol_state"], refused_sd["tool_result"]), ("ProtocolRefused", None))
                    self.assertIn(marker, refused_sd["failures"]); self.assertEqual(denied["child_before"], denied["child_after"])
                    if case == "aux_deterministic_wrong_zero":
                        self.assertEqual(sha(Path(control["aux_archive"])), prior["archive_post"]["sha256"])
                        self.assertEqual(len(prior["archive_operand_ledger_return"]["producers"]), 1)
                if case in ("aux_capture", "aux_return_fd"):
                    c = next(r for r in control["rows"] if r["stage"] == "C1"); r = calls[c["operation_id"]][1]
                    self.assertEqual(c["child_after"], c["child_before"] + 1); self.assertEqual(r["tool_result"], 0)
        _, _, _, foreign, calls, control = self.native_e8_ring_test_result("aux_object_nonzero")
        c = next(r for r in control["rows"] if r["stage"] == "C1"); receipt = calls[c["operation_id"]][1]
        self.assertEqual((receipt["protocol_state"], receipt["tool_result"], receipt["source_semantics"]["compiler_success"]), ("Completed", 7, False))
        following = control["rows"][-1]; self.assertEqual(following["child_before"], following["child_after"])
        self.assertEqual((calls[following["operation_id"]][1]["protocol_state"], calls[following["operation_id"]][1]["tool_result"]), ("ProtocolRefused", None))
        cuts = [row for row in control["rows"] if row["stage"] == "AfterC1Cut"]
        self.assertEqual(len(cuts), 1, control)
        cut = cuts[0]; cut_call, cut_receipt = calls[cut["operation_id"]]
        self.assertEqual((cut_call.name, cut_receipt["operation_id"]), (cut["operation_id"], cut["operation_id"]))
        self.assertEqual(sha(cut_call / "receipt.json"), cut["receipt_sha256"])
        cut_request = json.loads((cut_call / "request.json").read_bytes())
        self.assertEqual([os.fsdecode(bytes.fromhex(v)) for v in cut_request["args_hex"]], cut["argv"])
        self.assertTrue(all(cut_receipt[k] == v for k, v in cut_request.items()), cut_receipt)
        self.assertEqual((cut_request["role"], cut_receipt["role"], cut_receipt["protocol_state"], cut_receipt["tool_result"]),
                         ("ar", "ar", "ProtocolRefused", None))
        self.assertEqual(cut["child_before"], cut["child_after"])
        self.assertIn("ForeignFamilySticky", cut_receipt["failures"])
        self.assertIn("ForeignOperation:" + following["operation_id"] + ":ForeignProtocolSticky", foreign["blockers"])

    def test_native_e8_ring_aux_owned_bytes_survive_retained_and_request_only_cuts(self):
        for owned in ("object", "archive"):
            for kind, cut in (("source", "current"), ("extern", "retained"), ("output", "requestonly")):
                with self.subTest(owned=owned, kind=kind, cut=cut):
                    _, session, record, foreign, _, _ = self.native_e8_ring_test_result("aux_copy_" + kind + "_" + cut + "_" + owned)
                    control = json.loads((session / "e8-copy.json").read_bytes()); copy = Path(control["copy"])
                    self.assertEqual(sha(copy), control["sha256"]); self.assertIn(control["sha256"], foreign["quarantine"]["sha256"])
                    self.assertIn("NativeOutputRole:" + (control["cc_id"] if kind == "output" else str(copy)), record["blockers"])
                    self.assertFalse(any(r["path"] == str(copy) for r in record["consumed_sources"]))
                    self.assertFalse(any(control["cc_id"] in r["producers"] for r in record["extern_edges"]))
                    self.assertFalse(any(r["producer_invocation"] == control["cc_id"] for r in record["build_script_associations"]))
                    self.assertEqual(sha(session / "invocations" / control["cc_id"] / "receipt.json"), control["cc_receipt_sha256"])
                    if cut == "retained": self.assertNotIn(control["sha256"], {sha(p) for p in (session / "foreign-native-invocations").glob("*/*.raw")})
                    if cut == "requestonly": self.assertFalse((session / "foreign-native-invocations" / control["operation_id"] / "receipt.json").exists())
        for case in ("aux_source_pre", "aux_source_post"):
            with self.subTest(case=case):
                session, calls, control, negative = self.native_e8_ring_test_private_result(case)
                path = session / "vendor/ring/crypto/constant_time_test.c"; paths, _, hashes, _ = negative
                self.assertIn(str(path), paths); self.assertIn(sha(path), hashes); self.assertIn(owner.RING_AUX_SOURCE_SHA256, hashes)
                c = next(r for r in control["rows"] if r["stage"] == "C1"); r = calls[c["operation_id"]][1]
                self.assertEqual(c["child_after"], c["child_before"] + (1 if case.endswith("post") else 0))
                self.assertEqual((r["protocol_state"], r["tool_result"]), ("ProtocolRefused", 0 if case.endswith("post") else None))
                self.assertTrue(any("ForeignInputChanged" in x or "ForeignCompileSourcePin" in x for x in r["failures"]))
                following = control["rows"][-1]; self.assertEqual(following["child_before"], following["child_after"])
                self.assertEqual((calls[following["operation_id"]][1]["protocol_state"], calls[following["operation_id"]][1]["tool_result"]), ("ProtocolRefused", None))


    def test_native_compile_pin_stable_fd_read_rejects_real_changes_and_releases_fd(self):
        """Real borrowed file reads only; no native child, receipt or qualification."""
        import errno
        from unittest import mock

        data = b"a" * (65536 + 17)
        expected = hashlib.sha256(data).hexdigest()
        real_read = os.read
        cases = ("normal", "wrong_sha", "same_bytes_new_inode", "in_place_write", "extent_grow", "symlink", "hardlink", "io_failure")
        for case in cases:
            with self.subTest(case=case):
                directory = self.root / ("stable_pin_" + case); directory.mkdir()
                path = directory / "leaf"; path.write_bytes(data)
                replacement = directory / "replacement"
                before = path.stat(); original = directory / "original"
                if case == "same_bytes_new_inode": replacement.write_bytes(data)
                if case == "symlink": path.rename(original); path.symlink_to(original)
                if case == "hardlink": os.link(path, original)
                descriptors = set(); blocks = []; changed = False

                def read_once(fd, size):
                    nonlocal changed
                    descriptors.add(fd)
                    self.assertFalse(os.get_inheritable(fd))
                    if case == "io_failure": raise OSError(errno.EIO, "real pin read injected failure")
                    block = real_read(fd, size); blocks.append(len(block))
                    if block and not changed:
                        changed = True
                        if case == "same_bytes_new_inode":
                            os.replace(replacement, path)
                            self.assertNotEqual(path.stat().st_ino, before.st_ino)
                            self.assertEqual(path.read_bytes(), data)
                        elif case == "in_place_write":
                            # Change already-read bytes: a digest-only read would retain the old SHA.
                            with path.open("r+b") as writer:
                                writer.write(b"X"); writer.flush(); os.fsync(writer.fileno())
                            self.assertEqual(path.stat().st_ino, before.st_ino)
                        elif case == "extent_grow":
                            with path.open("ab") as writer:
                                writer.write(b"X"); writer.flush(); os.fsync(writer.fileno())
                    return block

                with mock.patch.object(owner.os, "read", read_once):
                    if case == "normal":
                        self.assertEqual(owner.foreign_compile_leaf_hash(path, expected), expected)
                    elif case == "io_failure":
                        with self.assertRaises(OSError) as raised:
                            owner.foreign_compile_leaf_hash(path, expected)
                        self.assertEqual(raised.exception.errno, errno.EIO)
                    else:
                        reason = "NotRegularFile" if case == "symlink" else "ForeignCompileSourcePin"
                        with self.assertRaisesRegex(owner.Refusal, "^" + reason + "$"):
                            owner.foreign_compile_leaf_hash(path, "0" * 64 if case == "wrong_sha" else expected)
                if case in ("symlink", "hardlink"):
                    self.assertEqual(descriptors, set())
                else:
                    self.assertEqual(len(descriptors), 1)
                    for fd in descriptors:
                        with self.assertRaises(OSError) as closed:
                            os.fstat(fd)
                        self.assertEqual(closed.exception.errno, errno.EBADF)
                if case == "normal": self.assertEqual(blocks, [65536, 17, 0])
                if case == "in_place_write":
                    self.assertEqual(blocks, [65536, 17, 0])
                    self.assertNotEqual(hashlib.sha256(path.read_bytes()).hexdigest(), expected)


    def test_native_e8_closed_seal_group_revalidation_and_first_fault(self):
        from unittest import mock
        _, session, record, foreign, calls, control = self.native_e8_ring_test_result("aux_fallback")
        self.assertFalse(any(b.startswith("ForeignOperation:") for b in foreign["blockers"]), foreign["blockers"])
        self.assertEqual(record["selected_library"], [])
        spec = importlib.util.spec_from_file_location("closed_ring_fixture_owner", self.tool)
        subject = importlib.util.module_from_spec(spec); spec.loader.exec_module(subject)
        policy = subject.strict_json(self.policy.read_bytes())
        build = subject.foreign_closed_ring_seal_scope
        main = sorted((ident, call, receipt) for ident, (call, receipt) in calls.items()
            if receipt.get("operation", {}).get("class") == "CompilerObjectCompile"
            and receipt["context"]["manifest"].endswith("/ring") and "ring_build" not in receipt["operation"])
        ident, call, receipt = main[0]; context = receipt["context"]
        source = Path(receipt["operation"]["source"]); tail = Path(control["aux_archive"])
        namespace = session / "foreign-native-invocations"
        self.assertEqual(len(main), 29)
        self.assertTrue(any(r.get("context", {}).get("manifest", "").endswith("/psm") for _, r in calls.values()))

        def replace(path, data):
            saved, before = path.read_bytes(), path.stat()
            path.chmod(before.st_mode | 0o200); path.write_bytes(data)
            def restore():
                path.write_bytes(saved); path.chmod(before.st_mode)
                os.utime(path, ns=(before.st_atime_ns, before.st_mtime_ns))
            return restore

        for _ in range(2):
            with mock.patch.object(subject, "foreign_ring_families", wraps=subject.foreign_ring_families) as families:
                resealed = subject.foreign_seal(session, policy)
            self.assertEqual(resealed, foreign)
            self.assertEqual(families.call_count, 1)  # Supplementary: each independent seal is still fresh.
            self.assertEqual((resealed["stage"], resealed["native_producer_qualification"]), ("StageCIncomplete", "not_issued"))

        for cut in ("source", "receipt", "stream", "tail", "alias", "new_call"):
            with self.subTest(cut=cut):
                restores, constructed = [], []
                def mutate(*args, **kwargs):
                    scope = build(*args, **kwargs); self.assertIsNotNone(scope)
                    constructed.append(scope)
                    if cut == "source": restores.append(replace(source, source.read_bytes() + b"\nTEST_CODE drift\n"))
                    elif cut == "receipt":
                        value = json.loads((call / "receipt.json").read_bytes()); value["tool_sha256"] = "0" * 64
                        restores.append(replace(call / "receipt.json", json.dumps(value).encode()))
                    elif cut == "stream": restores.append(replace(call / "stdout.raw", (call / "stdout.raw").read_bytes() + b"TEST_CODE drift"))
                    elif cut == "tail": restores.append(replace(tail, tail.read_bytes() + b"TEST_CODE drift"))
                    elif cut == "alias":
                        path = call / "stdout.raw"; backup = self.root / "TEST_CODE_closed_seal_stream_backup"
                        path.rename(backup); path.symlink_to(backup)
                        def restore_alias(): path.unlink(); backup.rename(path)
                        restores.append(restore_alias)
                    else:
                        pending = namespace / ("f" * 32); self.assertFalse(pending.exists()); pending.mkdir()
                        restores.append(pending.rmdir)
                    return scope
                try:
                    with mock.patch.object(subject, "foreign_closed_ring_seal_scope", side_effect=mutate):
                        rejected = subject.foreign_seal(session, policy)
                    self.assertEqual(len(constructed), 1)
                    marker = "NativeVersionChanged" if cut == "tail" else "ForeignClosedSealAlias" if cut == "alias" else "ForeignClosedSealPending" if cut == "new_call" else "ForeignClosedSealGenerationChanged"
                    self.assertIn("ForeignSeal:" + context["out_dir"] + ":" + marker, rejected["blockers"])
                    self.assertEqual((rejected["stage"], rejected["native_producer_qualification"], rejected["artifact_selection"]),
                                     ("StageCIncomplete", "not_issued", "not_observed"))
                    if cut == "source": self.assertIn("ForeignOperation:" + ident + ":ForeignCompileSourcePin", rejected["blockers"])
                    if cut == "tail": self.assertIn(str(tail), rejected["quarantine"]["paths"])
                    if cut == "stream": self.assertIn(sha(call / "stdout.raw"), rejected["quarantine"]["sha256"])
                    if cut == "new_call": self.assertTrue(any(b.startswith("ForeignNamespace:" + "f" * 32 + ":") for b in rejected["blockers"]))
                finally:
                    for restore in reversed(restores): restore()

        changed = dict(receipt); changed["family_return"] = {}
        restore = replace(call / "receipt.json", json.dumps(changed).encode())
        try:
            with mock.patch.object(subject, "foreign_closed_ring_seal_scope", return_value=None):
                original_order = subject.foreign_seal(session, policy)
            with mock.patch.object(subject, "foreign_closed_ring_seal_scope", wraps=build) as failed:
                fallback = subject.foreign_seal(session, policy)
            self.assertEqual(failed.call_count, 1)
            self.assertEqual(fallback, original_order)
            self.assertIn("ForeignOperation:" + ident + ":ForeignRingMainFamily", fallback["blockers"])
            self.assertFalse(any("ForeignClosedSeal" in b for b in fallback["blockers"]), fallback["blockers"])
        finally: restore()
        self.assertEqual(subject.foreign_seal(session, policy), foreign)
        self.assertEqual(sha(Path(control["main_archive"])), control["main_sha256"])
        self.assertEqual(record["selected_library"], [])


if __name__ == "__main__":
    unittest.main()
