const refreshBtn = document.querySelector("#refreshBtn");
const geocodeBtn = document.querySelector("#geocodeBtn");
const searchInput = document.querySelector("#searchInput");
const kindFilter = document.querySelector("#kindFilter");
const dateFilter = document.querySelector("#dateFilter");
const countryFilter = document.querySelector("#countryFilter");
const cityFilter = document.querySelector("#cityFilter");
const peopleFilter = document.querySelector("#peopleFilter");
const favoriteFilter = document.querySelector("#favoriteFilter");
const selectMode = document.querySelector("#selectMode");
const selectedCount = document.querySelector("#selectedCount");
const bulkCountry = document.querySelector("#bulkCountry");
const bulkCity = document.querySelector("#bulkCity");
const applyLocationBtn = document.querySelector("#applyLocationBtn");
const clearSelectionBtn = document.querySelector("#clearSelectionBtn");
const summary = document.querySelector("#summary");
const message = document.querySelector("#message");
const gallery = document.querySelector("#gallery");
const loadMoreBtn = document.querySelector("#loadMoreBtn");
const template = document.querySelector("#photoTemplate");
const viewer = document.querySelector("#viewer");
const viewerStage = document.querySelector("#viewerStage");
const viewerName = document.querySelector("#viewerName");
const viewerMeta = document.querySelector("#viewerMeta");
const viewerDetails = document.querySelector("#viewerDetails");
const favoriteCheckbox = document.querySelector("#favoriteCheckbox");
const closeBtn = document.querySelector("#closeBtn");
const prevBtn = document.querySelector("#prevBtn");
const nextBtn = document.querySelector("#nextBtn");
const rotateLeftBtn = document.querySelector("#rotateLeftBtn");
const rotateRightBtn = document.querySelector("#rotateRightBtn");

let photos = [];
let visiblePhotos = [];
let currentIndex = 0;
let renderLimit = 50;
let geocodePoll = null;
const pageSize = 50;
const selectedIds = new Set();

function showMessage(text, isError = false) {
  message.textContent = text;
  message.classList.toggle("error", isError);
}

async function api(path, options = {}) {
  const response = await fetch(`/api${path}`, options);

  if (!response.ok) {
    const data = await response.json().catch(() => ({}));
    throw new Error(data.error || "Error inesperado");
  }

  return response.json();
}

function fillSelect(select, values, defaultLabel) {
  const current = select.value;
  select.innerHTML = "";

  const all = document.createElement("option");
  all.value = "all";
  all.textContent = defaultLabel;
  select.appendChild(all);

  for (const value of values) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = value;
    select.appendChild(option);
  }

  if ([...select.options].some((option) => option.value === current)) {
    select.value = current;
  }
}

function updateCityOptions() {
  const country = countryFilter.value;
  const current = cityFilter.value;
  const cities = [...new Set(
    photos
      .filter((photo) => country !== "all" && photo.country === country && photo.city)
      .map((photo) => photo.city)
  )].sort();

  fillSelect(cityFilter, cities, "Todas las ciudades");
  cityFilter.disabled = country === "all";

  if ([...cityFilter.options].some((option) => option.value === current)) {
    cityFilter.value = current;
  }
}

function formatBytes(bytes) {
  if (bytes < 1024 * 1024) {
    return `${Math.round(bytes / 1024)} KB`;
  }

  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function formatDate(seconds) {
  if (!seconds) {
    return "sin fecha";
  }

  return new Date(seconds * 1000).toLocaleString();
}

function photoDate(photo) {
  if (photo.captured_at) {
    return photo.captured_at;
  }

  if (photo.modified_at) {
    return formatDate(photo.modified_at);
  }

  if (photo.date_bucket) {
    return photo.date_bucket;
  }

  return "sin fecha";
}

function addDetail(label, value) {
  if (value === null || value === undefined || value === "") {
    return;
  }

  const term = document.createElement("dt");
  const description = document.createElement("dd");
  term.textContent = label;
  description.textContent = value;
  viewerDetails.append(term, description);
}

function updateSelectionUi() {
  const count = selectedIds.size;
  selectedCount.textContent = `${count} seleccionada${count === 1 ? "" : "s"}`;
  clearSelectionBtn.disabled = count === 0;
  applyLocationBtn.disabled =
    count === 0 || (!bulkCountry.value.trim() && !bulkCity.value.trim());
}

function toggleSelection(id) {
  if (selectedIds.has(id)) {
    selectedIds.delete(id);
  } else {
    selectedIds.add(id);
  }

  updateSelectionUi();
  renderGallery();
}

function clearSelection() {
  selectedIds.clear();
  updateSelectionUi();
  renderGallery();
}

function applyFilters() {
  const term = searchInput.value.trim().toLowerCase();
  const kind = kindFilter.value;
  const date = dateFilter.value;
  const country = countryFilter.value;
  const city = cityFilter.disabled ? "all" : cityFilter.value;
  const person = peopleFilter.value;
  const favoritesOnly = favoriteFilter.checked;

  visiblePhotos = photos.filter((photo) => {
    const matchesTerm =
      !term ||
      photo.name.toLowerCase().includes(term) ||
      photo.relative_path.toLowerCase().includes(term);
    const matchesKind = kind === "all" || photo.kind === kind;
    const matchesDate = date === "all" || photo.date_bucket === date;
    const matchesCountry = country === "all" || photo.country === country;
    const matchesCity = city === "all" || photo.city === city;
    const matchesPerson =
      person === "all" ||
      (photo.people || "")
        .split(",")
        .map((name) => name.trim())
        .includes(person);
    const matchesFavorite = !favoritesOnly || photo.favorite === 1;
    return matchesTerm && matchesKind && matchesDate && matchesCountry && matchesCity && matchesPerson && matchesFavorite;
  });

  renderLimit = pageSize;
  renderGallery();
}

function renderGallery() {
  gallery.innerHTML = "";
  const renderedCount = Math.min(renderLimit, visiblePhotos.length);
  summary.textContent = `${renderedCount} visibles de ${visiblePhotos.length} filtrados (${photos.length} indexados)`;
  loadMoreBtn.hidden = renderedCount >= visiblePhotos.length;

  if (photos.length === 0) {
    gallery.innerHTML = '<p class="empty">No he encontrado fotos o videos compatibles.</p>';
    loadMoreBtn.hidden = true;
    return;
  }

  if (visiblePhotos.length === 0) {
    gallery.innerHTML = '<p class="empty">No hay resultados con ese filtro.</p>';
    loadMoreBtn.hidden = true;
    return;
  }

  const page = visiblePhotos.slice(0, renderLimit);

  for (const [index, photo] of page.entries()) {
    const node = template.content.firstElementChild.cloneNode(true);
    const thumb = node.querySelector(".thumb");
    const title = node.querySelector("strong");
    const meta = node.querySelector("small");
    node.classList.toggle("selected", selectedIds.has(photo.id));

    const date = photoDate(photo);
    title.textContent = date;
    const place = [photo.city, photo.country].filter(Boolean).join(", ");
    const people = photo.people ? ` - ${photo.people}` : "";
    const favorite = photo.favorite === 1 ? "Favorita" : "";
    meta.textContent = [favorite, place, photo.people].filter(Boolean).join(" - ");

    if (photo.kind === "video") {
      const video = document.createElement("video");
      video.src = photo.media_url;
      video.muted = true;
      video.preload = "none";
      thumb.appendChild(video);
      thumb.dataset.badge = "Video";
    } else if (photo.kind === "image") {
      const img = document.createElement("img");
      img.src = photo.extension === "heic" || photo.extension === "heif"
        ? photo.media_url
        : (photo.thumb_url || photo.media_url);
      img.alt = photo.name;
      img.loading = "lazy";
      thumb.appendChild(img);
      if (photo.extension === "heic" || photo.extension === "heif") {
        thumb.dataset.badge = "HEIC";
      }
    } else {
      thumb.textContent = photo.extension.toUpperCase();
    }

    const favoriteButton = document.createElement("span");
    favoriteButton.className = "favorite-toggle";
    favoriteButton.classList.toggle("is-favorite", photo.favorite === 1);
    favoriteButton.textContent = photo.favorite === 1 ? "♥" : "♡";
    favoriteButton.title = photo.favorite === 1 ? "Quitar de favoritas" : "Marcar favorita";
    favoriteButton.setAttribute("role", "button");
    favoriteButton.setAttribute("aria-label", favoriteButton.title);
    favoriteButton.addEventListener("click", async (event) => {
      event.preventDefault();
      event.stopPropagation();
      await toggleFavorite(photo.id, photo.favorite !== 1);
    });
    thumb.appendChild(favoriteButton);

    const selectBox = document.createElement("input");
    selectBox.type = "checkbox";
    selectBox.className = "select-toggle";
    selectBox.checked = selectedIds.has(photo.id);
    selectBox.hidden = !selectMode.checked;
    selectBox.setAttribute("aria-label", "Seleccionar foto");
    selectBox.addEventListener("click", (event) => {
      event.stopPropagation();
      toggleSelection(photo.id);
    });
    thumb.appendChild(selectBox);

    node.addEventListener("click", () => {
      if (selectMode.checked) {
        toggleSelection(photo.id);
        return;
      }

      openViewer(index);
    });
    gallery.appendChild(node);
  }
}

async function applyBulkLocation() {
  const country = bulkCountry.value.trim();
  const city = bulkCity.value.trim();

  if (selectedIds.size === 0 || (!country && !city)) {
    updateSelectionUi();
    return;
  }

  applyLocationBtn.disabled = true;
  showMessage("Actualizando lugar...");

  try {
    const payload = { ids: [...selectedIds] };
    if (country) {
      payload.country = country;
    }
    if (city) {
      payload.city = city;
    }

    const result = await api("/photos/bulk-location", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });

    selectedIds.clear();
    selectMode.checked = false;
    bulkCountry.value = "";
    bulkCity.value = "";
    updateSelectionUi();
    await loadPhotos();
    showMessage(`Lugar actualizado en ${result.updated} foto${result.updated === 1 ? "" : "s"}.`);
  } catch (error) {
    showMessage(error.message, true);
    updateSelectionUi();
  }
}

async function toggleFavorite(id, favorite) {
  const updated = await api(`/photos/${id}/favorite`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ favorite }),
  });

  const allIndex = photos.findIndex((item) => item.id === updated.id);
  if (allIndex >= 0) {
    photos[allIndex] = updated;
  }

  const visibleIndex = visiblePhotos.findIndex((item) => item.id === updated.id);
  if (visibleIndex >= 0) {
    visiblePhotos[visibleIndex] = updated;
  }

  if (favoriteFilter.checked && updated.favorite !== 1) {
    applyFilters();
    return updated;
  }

  renderGallery();
  return updated;
}

function openViewer(index) {
  currentIndex = index;
  renderViewer();
  viewer.showModal();
}

function renderViewer() {
  const photo = visiblePhotos[currentIndex];
  viewerStage.innerHTML = "";
  viewerDetails.innerHTML = "";
  const date = photoDate(photo);
  viewerName.textContent = date;
  const place = [photo.city, photo.country].filter(Boolean).join(", ");
  const people = photo.people ? ` - Personas: ${photo.people}` : "";
  const gps = photo.latitude && photo.longitude ? ` - GPS ${photo.latitude.toFixed(5)}, ${photo.longitude.toFixed(5)}` : "";
  viewerMeta.textContent = `${photo.extension.toUpperCase()} - ${formatBytes(photo.size_bytes)}${place ? ` - ${place}` : ""}${people}${gps}`;
  favoriteCheckbox.checked = photo.favorite === 1;
  const canRotate = photo.kind === "image" && ["jpg", "jpeg"].includes(photo.extension.toLowerCase());
  rotateLeftBtn.disabled = !canRotate;
  rotateRightBtn.disabled = !canRotate;

  addDetail("Archivo", photo.name);
  addDetail("Ruta", photo.relative_path);
  addDetail("Fecha", date);
  addDetail("Tipo", photo.kind === "video" ? "Video" : "Foto");
  addDetail("Formato", photo.extension.toUpperCase());
  addDetail("Tamaño", formatBytes(photo.size_bytes));
  addDetail("País", photo.country);
  addDetail("Ciudad", photo.city);
  addDetail("Personas", photo.people);
  addDetail("Favorita", photo.favorite === 1 ? "Sí" : "No");
  addDetail(
    "GPS",
    photo.latitude && photo.longitude
      ? `${photo.latitude.toFixed(6)}, ${photo.longitude.toFixed(6)}`
      : null
  );

  if (photo.kind === "video") {
    const video = document.createElement("video");
    video.src = photo.media_url;
    video.controls = true;
    video.autoplay = true;
    viewerStage.appendChild(video);
  } else {
    const img = document.createElement("img");
    img.src = photo.media_url;
    img.alt = photo.name;
    viewerStage.appendChild(img);
  }

  prevBtn.disabled = visiblePhotos.length <= 1;
  nextBtn.disabled = visiblePhotos.length <= 1;
}

function moveViewer(delta) {
  if (visiblePhotos.length === 0) {
    return;
  }

  currentIndex = (currentIndex + delta + visiblePhotos.length) % visiblePhotos.length;
  renderViewer();
}

async function rotatePhoto(direction) {
  const photo = visiblePhotos[currentIndex];
  if (!photo) {
    return;
  }

  rotateLeftBtn.disabled = true;
  rotateRightBtn.disabled = true;
  showMessage("Girando foto original...");

  try {
    const updated = await api(`/photos/${photo.id}/rotate`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ direction }),
    });
    const stamp = Date.now();
    updated.media_url = `${updated.media_url}?v=${stamp}`;
    updated.thumb_url = `${updated.thumb_url}?v=${stamp}`;

    const allIndex = photos.findIndex((item) => item.id === updated.id);
    if (allIndex >= 0) {
      photos[allIndex] = updated;
    }

    const visibleIndex = visiblePhotos.findIndex((item) => item.id === updated.id);
    if (visibleIndex >= 0) {
      visiblePhotos[visibleIndex] = updated;
      currentIndex = visibleIndex;
    }

    renderViewer();
    renderGallery();
    showMessage("Foto girada.");
  } catch (error) {
    showMessage(error.message, true);
    renderViewer();
  }
}

async function loadPhotos() {
  showMessage("Leyendo indice...");
  if (refreshBtn) {
    refreshBtn.disabled = true;
  }

  try {
    const loadedPhotos = await api("/photos");
    const filters = await api("/filters");
    photos = loadedPhotos;
    fillSelect(dateFilter, filters.dates, "Todas las fechas");
    fillSelect(countryFilter, filters.countries, "Todos los paises");
    fillSelect(peopleFilter, filters.people, "Todas las personas");
    updateCityOptions();
    applyFilters();
    showMessage("");
  } catch (error) {
    photos = [];
    visiblePhotos = [];
    renderGallery();
    showMessage(error.message, true);
  } finally {
    if (refreshBtn) {
      refreshBtn.disabled = false;
    }
  }
}

async function refreshIndex() {
  showMessage("Actualizando indice desde Z:\\Fotos_iphone...");
  if (refreshBtn) {
    refreshBtn.disabled = true;
  }

  try {
    const result = await api("/refresh", { method: "POST" });
    showMessage(`Indice actualizado: ${result.indexed} archivos.`);
    await loadPhotos();
  } catch (error) {
    showMessage(error.message, true);
  } finally {
    if (refreshBtn) {
      refreshBtn.disabled = false;
    }
  }
}

async function geocodeLocations() {
  showMessage("Iniciando barrido de pais y ciudad...");
  if (geocodeBtn) {
    geocodeBtn.disabled = true;
  }

  try {
    const result = await api("/geocode", { method: "POST" });
    showMessage(result.message);
    startGeocodePolling();
  } catch (error) {
    showMessage(error.message, true);
    if (geocodeBtn) {
      geocodeBtn.disabled = false;
    }
  }
}

function startGeocodePolling() {
  if (geocodePoll) {
    clearInterval(geocodePoll);
  }

  geocodePoll = setInterval(checkGeocodeStatus, 3000);
  checkGeocodeStatus();
}

async function checkGeocodeStatus() {
  try {
    const status = await api("/geocode/status");
    if (status.running) {
      showMessage(`Asignando lugares: ${status.message}`);
      if (geocodeBtn) {
        geocodeBtn.disabled = true;
      }
      return;
    }

    clearInterval(geocodePoll);
    geocodePoll = null;
    if (geocodeBtn) {
      geocodeBtn.disabled = false;
    }
    showMessage(status.message);
    await loadPhotos();
  } catch (error) {
    clearInterval(geocodePoll);
    geocodePoll = null;
    if (geocodeBtn) {
      geocodeBtn.disabled = false;
    }
    showMessage(error.message, true);
  } finally {
  }
}

refreshBtn?.addEventListener("click", refreshIndex);
geocodeBtn?.addEventListener("click", geocodeLocations);
searchInput.addEventListener("input", applyFilters);
kindFilter.addEventListener("change", applyFilters);
dateFilter.addEventListener("change", applyFilters);
countryFilter.addEventListener("change", () => {
  updateCityOptions();
  applyFilters();
});
cityFilter.addEventListener("change", applyFilters);
peopleFilter.addEventListener("change", applyFilters);
favoriteFilter.addEventListener("change", applyFilters);
selectMode.addEventListener("change", () => {
  if (!selectMode.checked) {
    selectedIds.clear();
  }

  updateSelectionUi();
  renderGallery();
});
bulkCountry.addEventListener("input", updateSelectionUi);
bulkCity.addEventListener("input", updateSelectionUi);
applyLocationBtn.addEventListener("click", applyBulkLocation);
clearSelectionBtn.addEventListener("click", clearSelection);
closeBtn.addEventListener("click", () => viewer.close());
viewer.addEventListener("click", (event) => {
  if (event.target === viewer) {
    viewer.close();
  }
});
prevBtn.addEventListener("click", () => moveViewer(-1));
nextBtn.addEventListener("click", () => moveViewer(1));
rotateLeftBtn.addEventListener("click", () => rotatePhoto("left"));
rotateRightBtn.addEventListener("click", () => rotatePhoto("right"));
favoriteCheckbox.addEventListener("change", async () => {
  const photo = visiblePhotos[currentIndex];
  if (!photo) {
    return;
  }

  try {
    const updated = await toggleFavorite(photo.id, favoriteCheckbox.checked);
    const newIndex = visiblePhotos.findIndex((item) => item.id === updated.id);
    if (newIndex >= 0) {
      currentIndex = newIndex;
    }
    renderViewer();
  } catch (error) {
    showMessage(error.message, true);
    favoriteCheckbox.checked = photo.favorite === 1;
  }
});
loadMoreBtn.addEventListener("click", () => {
  renderLimit += pageSize;
  renderGallery();
});

document.addEventListener("keydown", (event) => {
  if (!viewer.open) {
    return;
  }

  if (event.key === "ArrowLeft") {
    moveViewer(-1);
  }

  if (event.key === "ArrowRight") {
    moveViewer(1);
  }
});

loadPhotos();
