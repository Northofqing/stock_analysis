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
 'deny': ['--deny=unused'],
 'forbid': ['--forbid=unused'],
 'force_warn': ['--force-warn=unused'],
 'response': ['@response'],
 'unknown_codegen': ['-C', 'TEST_CODE_unknown=yes'],
 'unstable': ['-Zrandomize-layout']}

JOBSERVER_MUTATIONS = (
    "mismatch", "reversed", "duplicate", "stdio", "negative", "overflow", "noncanonical",
    "closed", "regular", "empty", "fifo", "extra", "whitespace", "makeflags", "mflags",
)

FAKE_RUSTC = r'''
import errno, fcntl, json, os, pathlib, stat, sys
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
    executable.chmod(0o700)
else:
    for suffix in ['.rmeta','.rlib']:
        (out/('lib'+name+suffix)).write_bytes(name.encode()+b':'+body)
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
    if MODE=='wrong_sysroot' and name=='dep': command[2:2]=['--sysroot','/TEST_CODE_wrong_sysroot']
    if MODE=='lint_equals':
        command[2:2]=LINTS[name]+(['--target','x86_64-apple-darwin'] if name=='stock_analysis' else [])
    if MODE.startswith('lint_invalid:') and name=='dep': command[2:2]=ARG_CASES[MODE.split(':',1)[1]]
    with jobserver(env,name) as (child,descriptors):
        result=subprocess.run(command,env=child,pass_fds=descriptors)
    if result.returncode: emit({'reason':'build-finished','success':False});sys.exit(result.returncode)
    if kind=='bin':
        alias=out/'build-script-build';shutil.copy2(out/name,alias);files=[str(alias)];executable=str(alias)
    else: files=[str(out/('lib'+name+'.rlib')),str(out/('lib'+name+'.rmeta'))];executable=None
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
    compile('stock_analysis',app/'src/lib.rs',app,'rlib','TEST_CODE_app',target/'deps',
            extra=['--target','x86_64-apple-darwin']+(['--extern','probe='+str(probe/'libproc_macro2.rmeta')] if case=='transient_extern' else []),env_extra={'OUT_DIR':str(out),'FIXTURE_GENERATED_NAME':generated_name})
    if case in ('snapshot_tamper','snapshot_missing'):
        snapshot=next((session/'invocations').glob('*/probe-output-*.raw'))
        if case=='snapshot_tamper': snapshot.write_bytes(b'TEST_CODE_CHANGED')
        else: snapshot.unlink()
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
        if mode.startswith("nested:"):
            write(sysroot / "lib/rustlib/x86_64-apple-darwin/lib/test.bin", "TEST_CODE_HOST_SYSROOT")
            for name, version in (("libc", "0.2.184"), ("proc-macro2", "1.0.106")):
                write(vendor / name / "Cargo.toml", '[package]\nname="' + name + '"\nversion="' + version + '"\n')
                write(vendor / name / "build.rs", "// TEST_CODE generator simulation for " + name + "\n")
                for probe in ("proc_macro_span", "proc_macro_span_file", "proc_macro_span_location"):
                    write(vendor / name / ("src/probe/" + probe + ".rs"), "// TEST_CODE " + probe + "\n")

        rustc = write(self.root / "fake-rustc", "#!" + PYTHON + " -I\n" + FAKE_RUSTC)
        cargo_source = FAKE_CARGO.replace("compile('dep',session/", NESTED_CARGO_FLOW + "\ncompile('dep',session/", 1)
        cargo = write(self.root / "fake-cargo", "#!" + PYTHON + " -I\n" + cargo_source.replace("__MODE__", repr(mode)).replace("__LINTS__", repr(OBSERVED_LINTS)).replace("__ARG_CASES__", repr(LINT_MUTATIONS)))
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


if __name__ == "__main__":
    unittest.main()
