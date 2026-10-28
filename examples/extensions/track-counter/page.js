// The page side of the track counter: asks its WebAssembly for the count and logs every answer.
ft.on('native', (data) => ft.log('tracks played:', data.count));
ft.native.send({ get: true }).catch(ft.error);
