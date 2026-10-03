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
def value(key): return args[args.index(key)+1]
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
source=pathlib.Path(args[-1]); body=source.read_bytes()
if name=='build_script_build':
    executable=out/name
    executable.write_text("#!"+sys.executable+chr(10)+'import os,pathlib'+chr(10)+'p=pathlib.Path(os.environ["OUT_DIR"]);p.mkdir(parents=True,exist_ok=True);(p/"generated.rs").write_text("pub const FIXTURE: u8 = 1;")'+chr(10))
    executable.chmod(0o700)
else:
    for suffix in ['.rmeta','.rlib']:
        (out/('lib'+name+suffix)).write_bytes(name.encode()+b':'+body)
inputs=[str(source)]
if name=='stock_analysis': inputs.append(str(pathlib.Path(os.environ['OUT_DIR'])/'generated.rs'))
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
    emit({'reason':'compiler-artifact','package_id':package,'target':{'src_path':str(root),'kind':['custom-build'] if kind=='bin' else ['lib'],'name':name},'filenames':files,'executable':executable,'fresh':False})
    return executable
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
        rustc = write(self.root / "fake-rustc", "#!" + PYTHON + " -I\n" + FAKE_RUSTC)
        cargo = write(self.root / "fake-cargo", "#!" + PYTHON + " -I\n" + FAKE_CARGO.replace("__MODE__", repr(mode)).replace("__LINTS__", repr(OBSERVED_LINTS)).replace("__ARG_CASES__", repr(LINT_MUTATIONS)))
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
