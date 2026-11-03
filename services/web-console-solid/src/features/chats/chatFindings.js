// The one turn-detail loader the chat views share.
//
// A thread and the workbox's live view show the same turns; a finding loaded in
// one is already there in the other, which is why the queue is per module and
// not per component.

import * as chatsApi from '../../api/chats.js';
import { createFindingsQueue } from './findingsQueue.js';

export const turnDetails = createFindingsQueue({ load: (id) => chatsApi.turn(id) });
