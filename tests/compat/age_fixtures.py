"""Four fresh processes prove both age key types across Node and native Rust."""
import hashlib
import json

from reference import HERE, ROOT, execute
from runner import check_json

AGE_CASES = [
    'AG01-encoding-utf8-views-fatal-stream-and-bom',
    'AG02-stream-backpressure-cancel-and-errors',
    'AG03-response-bodies-once-offsets-and-errors',
    'AG04-system-random-quota-types-and-views',
    'AG05-generate-x25519-and-hybrid-derive-and-normalize',
    'AG06-armor-roundtrip-unicode-view-and-chunk-boundary',
    'AG07-non-armor-preserves-types-and-identity',
    'AG08-validation-wrong-keys-tampering-and-recovery',
    'AG09-secret-redaction-and-empty-config',
    'AG10-public-age-in-actual-async-script-operator',
    'AG11-missing-runtime-diagnostics-before-crypto',
]
TYPES = ['x25519', 'mlkem768-x25519']
PAYLOADS = [('empty', ''), ('unicode', '订阅🙂\r\n\0尾\n'), ('view', '视图🙂\0\r\n'),
            ('chunk-boundary', 'a' * 65535 + '🙂尾')]


def check_age_run(result, progress, *, full, directions):
    """Validate exact assertions and plaintext hashes independently of JS."""
    check_json(result['passed'], AGE_CASES if full else [])
    check_json(list(result['observations']), AGE_CASES if full else [])
    check_json(progress, [{'id': case, 'event': event} for case in (AGE_CASES if full else [])
                          for event in ('start', 'end')])
    expected = [{'direction': direction, 'type': kind, 'id': name,
                 'utf8Length': len(text.encode()), 'md5': hashlib.md5(text.encode()).hexdigest()}
                for direction in directions for kind in TYPES for name, text in PAYLOADS]
    check_json(result['verified'], expected)


def run_age_probe(node, executable, environment, output, report, container=None):
    report['age_runs'] = []
    exchanges = {}
    observations = None
    for name, kind, mode, peer_name, own_name, directions in [
        ('node-age-generate', 'node', 'full', None, None, []),
        ('rust-age-exchange', 'rust', 'full', 'node-age-generate', None, ['node-key-to-rust']),
        ('node-age-reply', 'node', 'reply', 'rust-age-exchange', 'node-age-generate', ['rust-key-to-node', 'rust-to-node-key']),
        ('rust-age-finish', 'rust', 'finish', 'node-age-reply', 'rust-age-exchange', ['node-to-rust-key']),
    ]:
        state = output / (name + '-state')
        state.mkdir()
        for filename in ('sub-store.json', 'sub-store-probe.json', 'root.json'):
            (state / filename).write_text('{}\n', encoding='utf-8')
        value = {'mode': mode}
        for field, key in (('peer', peer_name), ('own', own_name)):
            if key:
                value[field] = exchanges[key]
        (state / 'age-input.json').write_text(json.dumps(value, ensure_ascii=False) + '\n', encoding='utf-8')
        child_env = {**environment, 'SUB_STORE_COMPAT_STATE_DIR': str(state), 'SUB_STORE_DATA_BASE_PATH': str(state),
                     'SUB_STORE_COMPAT_UPSTREAM': 'http://127.0.0.1:1'}
        if kind == 'rust' and container:
            process = container(state, ['--age'], child_env, name)
        else:
            argv = [node, '--require', HERE / 'register.cjs', HERE / 'age-reference.cjs'] if kind == 'node' else [executable, '--age']
            process = execute(argv, ROOT / 'backend' if kind == 'node' else ROOT, child_env, output, name, timeout=60,
                              direct_reason='Direct age process ownership allows watchdog cleanup on Windows')
        progress = [json.loads(line.removeprefix('COMPAT_AGE ')) for line in process.stderr.splitlines()
                    if line.startswith('COMPAT_AGE ')]
        item = {'name': name, 'exit_code': process.returncode, 'progress': progress}
        report['age_runs'].append(item)
        if process.returncode:
            raise RuntimeError(f'{name} failed; inspect preserved logs')
        lines = process.stdout.splitlines()
        result = json.loads(next(line.removeprefix('COMPAT_AGE_RESULT ') for line in lines if line.startswith('COMPAT_AGE_RESULT '))
                            if kind == 'node' else lines[-1])
        item['result'] = result
        check_age_run(result, progress, full=mode == 'full', directions=directions)
        if mode == 'full':
            if observations is None:
                observations = result['observations']
            check_json(result['observations'], observations)
        exchange_file = state / 'age-output.json'
        exchanges[name] = json.loads(exchange_file.read_text(encoding='utf-8'))
        check_json(exchanges[name]['sender'], kind)
        item['exchange_sha256'] = hashlib.sha256(exchange_file.read_bytes()).hexdigest()
        item['input_sha256'] = hashlib.sha256((state / 'age-input.json').read_bytes()).hexdigest()
        item['command'] = json.loads((output / f'{name}.command.json').read_text(encoding='utf-8'))
    report['age_interoperability'] = {'key_types': TYPES, 'fresh_processes': 4, 'verified_plaintexts': 32,
                                    'fixture_secrets': 'Owned age-*-state exchange files only; never production credentials'}
    report['age_artifact_check'] = check_age_artifacts(output, report)


def check_age_artifacts(directory, report):
    """Independently join raw logs, immutable files and all four exchange stages."""
    runs = report['age_runs']
    names = ['node-age-generate', 'rust-age-exchange', 'node-age-reply', 'rust-age-finish']
    check_json([item['name'] for item in runs], names)
    check_json(report['age_interoperability'], {'key_types': TYPES, 'fresh_processes': 4, 'verified_plaintexts': 32,
        'fixture_secrets': 'Owned age-*-state exchange files only; never production credentials'})
    directions = [[], ['node-key-to-rust'], ['rust-key-to-node', 'rust-to-node-key'], ['node-to-rust-key']]
    exchanges, inputs, logs = [], [], []
    for index, item in enumerate(runs):
        name = names[index]
        check_json(item['exit_code'], 0)
        command = json.loads((directory / (name + '.command.json')).read_text(encoding='utf-8'))
        check_json(command, item['command'])
        check_json(command['exit_code'], 0)
        check_json(command['timed_out'], False)
        stdout = (directory / (name + '.stdout.log')).read_text(encoding='utf-8')
        stderr = (directory / (name + '.stderr.log')).read_text(encoding='utf-8')
        progress = [json.loads(line.removeprefix('COMPAT_AGE ')) for line in stderr.splitlines() if line.startswith('COMPAT_AGE ')]
        check_json(progress, item['progress'])
        result = json.loads(next(line.removeprefix('COMPAT_AGE_RESULT ') for line in stdout.splitlines()
                                if line.startswith('COMPAT_AGE_RESULT ')) if index % 2 == 0 else stdout.splitlines()[-1])
        check_json(result, item['result'])
        check_age_run(result, progress, full=index < 2, directions=directions[index])
        state = directory / (name + '-state')
        for filename, digest in [('age-input.json', 'input_sha256'), ('age-output.json', 'exchange_sha256')]:
            check_json(hashlib.sha256((state / filename).read_bytes()).hexdigest(), item[digest])
        inputs.append(json.loads((state / 'age-input.json').read_text(encoding='utf-8')))
        exchanges.append(json.loads((state / 'age-output.json').read_text(encoding='utf-8')))
        check_json(exchanges[-1]['sender'], 'node' if index % 2 == 0 else 'rust')
        logs.append(stdout + stderr)
    check_json(runs[1]['result']['observations'], runs[0]['result']['observations'])
    check_json(inputs, [{'mode': 'full'}, {'mode': 'full', 'peer': exchanges[0]},
                       {'mode': 'reply', 'peer': exchanges[1], 'own': exchanges[0]},
                       {'mode': 'finish', 'peer': exchanges[2], 'own': exchanges[1]}])
    secrets, publics, ciphertexts = [], [], []
    for index, exchange in enumerate(exchanges):
        check_json(sorted(exchange), sorted([['sender', 'pairs'], ['sender', 'pairs', 'replies'],
                                             ['sender', 'replies'], ['sender']][index]))
        if 'pairs' in exchange:
            check_json([pair['type'] for pair in exchange['pairs']], TYPES)
            for kind, pair in zip(TYPES, exchange['pairs']):
                assert pair['secret'].startswith('AGE-SECRET-KEY-1' if kind == TYPES[0] else 'AGE-SECRET-KEY-PQ-1')
                assert pair['public'].startswith('age1' if kind == TYPES[0] else 'age1pq1')
                secrets.append(pair['secret']); publics.append(pair['public'])
        if 'replies' in exchange:
            check_json([pair['type'] for pair in exchange['replies']], TYPES)
            check_json([pair['public'] for pair in exchange['replies']], [pair['public'] for pair in exchanges[1 - (index % 2)]['pairs']])
        for pair in exchange.get('pairs', []) + exchange.get('replies', []):
            check_json([item['id'] for item in pair['items']], [name for name, _ in PAYLOADS])
            for item in pair['items']:
                assert item['armor'].startswith('-----BEGIN AGE ENCRYPTED FILE-----\n')
                assert item['armor'].rstrip().endswith('-----END AGE ENCRYPTED FILE-----')
                ciphertexts.append(item['armor'])
    assert len(secrets) == len(set(secrets)) == len(set(publics)) == 4
    assert len(ciphertexts) == len(set(ciphertexts)) == 32
    assert not any(secret in log for secret in secrets for log in logs), 'Fixture identities leaked into console logs'
    return {'groups_per_runtime': len(AGE_CASES), 'fresh_processes': 4, 'verified_plaintexts': 32,
            'independent_identities': 4, 'unique_ciphertexts': 32, 'raw_progress_events': 4 * len(AGE_CASES)}
