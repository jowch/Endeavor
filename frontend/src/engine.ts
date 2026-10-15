// Which notebook system's page this is. Ember's (R notebooks) is a fork of
// Pluto's, so the same script works on both, except where Ember differs
// (Ember #62): the core serves its pages under /ember/.

export const onEmber = (): boolean => location.pathname.startsWith("/ember/");
