// Le « service worker » du tableau de bord : il reçoit les notifications
// (Web Push) même quand la page est fermée, et les affiche.
//
// Il ne met rien en cache et ne voit aucune donnée de la maison : il ne
// connaît que ce que le serveur lui envoie, chiffré pour ce seul navigateur.
// Un bouton (« Valider », ou celui d'une automatisation) renvoie au serveur
// le ticket reçu avec la notification : à usage unique, pour ce seul message.
"use strict";

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

const text = (value, fallback) => (typeof value === "string" && value !== "" ? value : fallback);

/** Une notification, avec l'icône et la langue de la maison. */
function show(title, options) {
  return self.registration.showNotification(title, { icon: "/icon-192.png", badge: "/icon-192.png", lang: "fr", ...options });
}

self.addEventListener("push", (event) => {
  let data = {};
  try {
    data = event.data ? event.data.json() : {};
  } catch {
    // Contenu illisible : on affiche quand même quelque chose (obligatoire).
  }
  // Seules les adresses de ce tableau de bord sont ouvertes au toucher.
  const url = typeof data.url === "string" && data.url.startsWith("/") && !data.url.startsWith("//") ? data.url : "/";
  // Les boutons n'existent qu'avec un ticket ; deux au plus.
  const token = typeof data.token === "string" && /^[A-Za-z0-9_-]{1,100}$/.test(data.token) ? data.token : null;
  const actions = token && Array.isArray(data.actions)
    ? data.actions
      .filter((a) => typeof a?.action === "string" && typeof a?.title === "string")
      .slice(0, 2)
      .map((a) => ({ action: a.action, title: a.title }))
    : [];
  event.waitUntil(show(text(data.title, "Maison"), {
    body: text(data.body, ""),
    tag: text(data.tag, "maison"),
    // Une nouvelle notification du même sujet refait sonner le téléphone.
    renotify: true,
    actions,
    data: { url, token: actions.length > 0 ? token : null },
  }));
});

/** Un bouton touché : le serveur agit, et la réponse remplace la notification. */
async function press(notification, action) {
  let message = "Impossible de joindre la maison. Ouvre le tableau de bord.";
  try {
    const response = await fetch("/api/push/action", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ token: notification.data.token, action }),
      credentials: "omit",
      cache: "no-store",
    });
    const body = await response.json().catch(() => ({}));
    const said = text(body.message, text(body.error, ""));
    if (said) message = said.charAt(0).toUpperCase() + said.slice(1);
  } catch {
    // Hors du réseau de la maison (sans VPN) : le message ci-dessus.
  }
  await show(notification.title, {
    body: message,
    tag: notification.tag,
    // La réponse ne refait pas sonner le téléphone.
    renotify: false,
    silent: true,
    data: { url: notification.data.url, token: null },
  });
}

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const url = event.notification.data?.url ?? "/";
  if (event.action && event.notification.data?.token) {
    event.waitUntil(press(event.notification, event.action));
    return;
  }
  event.waitUntil((async () => {
    const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
    for (const client of windows) {
      if (new URL(client.url).origin !== self.location.origin) continue;
      await client.focus();
      client.postMessage({ type: "open", url });
      return;
    }
    await self.clients.openWindow(url);
  })());
});
