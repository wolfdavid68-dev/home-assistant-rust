// Le « service worker » du tableau de bord : il reçoit les notifications
// (Web Push) même quand la page est fermée, et les affiche.
//
// Il ne met rien en cache et ne voit aucune donnée de la maison : il ne
// connaît que ce que le serveur lui envoie, chiffré pour ce seul navigateur.
"use strict";

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

self.addEventListener("push", (event) => {
  let data = {};
  try {
    data = event.data ? event.data.json() : {};
  } catch {
    // Contenu illisible : on affiche quand même quelque chose (obligatoire).
  }
  const text = (value, fallback) => (typeof value === "string" && value !== "" ? value : fallback);
  // Seules les adresses de ce tableau de bord sont ouvertes au toucher.
  const url = typeof data.url === "string" && data.url.startsWith("/") && !data.url.startsWith("//") ? data.url : "/";
  event.waitUntil(self.registration.showNotification(text(data.title, "Maison"), {
    body: text(data.body, ""),
    tag: text(data.tag, "maison"),
    // Une nouvelle notification du même sujet refait sonner le téléphone.
    renotify: true,
    icon: "/icon-192.png",
    badge: "/icon-192.png",
    lang: "fr",
    data: { url },
  }));
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const url = event.notification.data?.url ?? "/";
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
