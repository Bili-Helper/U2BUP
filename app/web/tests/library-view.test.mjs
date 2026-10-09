import assert from 'node:assert/strict';
import {test} from 'node:test';
import {createLibraryViewLoader} from '../src/library-view.ts';

function pending() {
  let resolve, reject;
  const promise = new Promise((a, b) => { resolve = a; reject = b; });
  return {promise, resolve, reject};
}

test('switching libraries keeps the newer view when responses arrive out of order', async () => {
  const first = pending(), second = pending(), applied = [];
  const loader = createLibraryViewLoader(id => id === 'LiveRec' ? first.promise : second.promise,
    (snapshot, id) => applied.push({snapshot, id}));
  const oldRequest = loader.load('LiveRec');
  loader.invalidate();
  const currentRequest = loader.load('biliLiveTools');
  second.resolve({assets: ['merged.mp4']});
  assert.equal(await currentRequest, true);
  first.resolve({assets: ['old.flv']});
  assert.equal(await oldRequest, false);
  assert.deepEqual(applied, [{snapshot: {assets: ['merged.mp4']}, id: 'biliLiveTools'}]);
});

test('deleting a library invalidates its in-flight view and failed scans keep the saved view', async () => {
  const request = pending(), applied = [];
  const loader = createLibraryViewLoader(() => request.promise, value => applied.push(value));
  const loading = loader.load('deleted-library');
  loader.invalidate();
  request.resolve({assets: ['deleted.mp4']});
  assert.equal(await loading, false);
  assert.deepEqual(applied, []);

  const failing = createLibraryViewLoader(async () => { throw new Error('offline'); }, value => applied.push(value));
  await assert.rejects(failing.load('unavailable-library'), /offline/);
  assert.deepEqual(applied, []);
});
