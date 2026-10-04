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


if __name__ == "__main__":
    unittest.main()
