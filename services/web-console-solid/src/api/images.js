// Image registry, assignments and the images x nodes matrix (ADR 0048).

import { request } from './client.js';

export const registry = () => request('registry/images');
export const matrix = () => request('images/matrix');
export const assign = (body) => request('image-assignments', { method: 'POST', body });
export const unassign = (id) => request(`image-assignments/${id}`, { method: 'DELETE' });
