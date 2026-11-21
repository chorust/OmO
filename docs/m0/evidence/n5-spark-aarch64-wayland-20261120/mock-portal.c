#include <gio/gio.h>
#include <stdio.h>
#include <string.h>

static GDBusConnection *connection;
static GMainLoop *loop;
static gboolean cancel_mode;
static unsigned request_id;
static GDBusNodeInfo *node;
static const char *request_xml =
  "<node><interface name='org.freedesktop.portal.Request'>"
  "<method name='Close'/>"
  "<signal name='Response'><arg type='u'/><arg type='a{sv}'/></signal>"
  "</interface></node>";

static const char *portal_xml =
  "<node><interface name='org.freedesktop.portal.FileChooser'>"
  "<method name='OpenFile'>"
  "<arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='a{sv}' direction='in'/><arg type='o' direction='out'/>"
  "</method>"
  "<method name='SaveFile'>"
  "<arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='a{sv}' direction='in'/><arg type='o' direction='out'/>"
  "</method>"
  "<property name='version' type='u' access='read'/>"
  "</interface></node>";

static gboolean emit_cancel(gpointer data) {
  const char *path = data;
  GVariantBuilder builder;
  GError *error = NULL;
  g_variant_builder_init(&builder, G_VARIANT_TYPE_VARDICT);
  if (!g_dbus_connection_emit_signal(connection, NULL, path,
      "org.freedesktop.portal.Request", "Response",
      g_variant_new("(u@a{sv})", 1, g_variant_builder_end(&builder)), &error)) {
    g_printerr("PORTAL_SIGNAL_ERROR %s\n", error->message);
    g_clear_error(&error);
  } else {
    g_printerr("PORTAL_RESPONSE_CANCELLED %s\n", path);
  }
  g_free((gpointer)path);
  return G_SOURCE_REMOVE;
}

static void request_method(GDBusConnection *conn, const gchar *sender,
    const gchar *object_path, const gchar *interface_name,
    const gchar *method_name, GVariant *parameters,
    GDBusMethodInvocation *invocation, gpointer user_data) {
  (void)conn; (void)sender; (void)object_path; (void)interface_name;
  (void)parameters; (void)user_data;
  if (g_str_equal(method_name, "Close")) {
    g_dbus_method_invocation_return_value(invocation, NULL);
  }
}

static GDBusInterfaceVTable request_vtable = { request_method, NULL, NULL, {0} };

static void portal_method(GDBusConnection *conn, const gchar *sender,
    const gchar *object_path, const gchar *interface_name,
    const gchar *method_name, GVariant *parameters,
    GDBusMethodInvocation *invocation, gpointer user_data) {
  (void)conn; (void)sender; (void)object_path; (void)interface_name;
  (void)parameters; (void)user_data;
  g_printerr("PORTAL_METHOD %s\n", method_name);
  if (!cancel_mode) {
    g_dbus_method_invocation_return_dbus_error(invocation,
      "org.freedesktop.portal.Error.Failed",
      "mock desktop portal rejected the file chooser request");
    return;
  }
  char *path = g_strdup_printf("/org/freedesktop/portal/desktop/request/mock/r%u", ++request_id);
  GError *error = NULL;
  GDBusInterfaceInfo *request_info = g_dbus_node_info_lookup_interface(node, "org.freedesktop.portal.Request");
  guint registration = g_dbus_connection_register_object(connection, path, request_info,
      &request_vtable, NULL, NULL, &error);
  if (!registration) {
    g_printerr("PORTAL_REGISTER_ERROR %s\n", error->message);
    g_dbus_method_invocation_return_gerror(invocation, error);
    g_clear_error(&error);
    g_free(path);
    return;
  }
  g_dbus_method_invocation_return_value(invocation, g_variant_new("(o)", path));
  g_timeout_add(50, emit_cancel, path);
}

static GVariant *portal_get_property(GDBusConnection *conn,
    const gchar *sender, const gchar *object_path,
    const gchar *interface_name, const gchar *property_name,
    GError **error, gpointer user_data) {
  (void)conn; (void)sender; (void)object_path; (void)interface_name;
  (void)error; (void)user_data;
  if (g_str_equal(property_name, "version")) return g_variant_new_uint32(4);
  return NULL;
}

static GDBusInterfaceVTable portal_vtable = { portal_method, portal_get_property, NULL, {0} };

static void name_acquired(GDBusConnection *conn, const gchar *name, gpointer data) {
  (void)conn; (void)data;
  g_printerr("PORTAL_NAME_ACQUIRED %s\n", name);
}

int main(int argc, char **argv) {
  if (argc != 2 || (strcmp(argv[1], "error") != 0 && strcmp(argv[1], "cancel") != 0)) {
    fprintf(stderr, "usage: %s error|cancel\n", argv[0]); return 2;
  }
  cancel_mode = strcmp(argv[1], "cancel") == 0;
  GError *error = NULL;
  node = g_dbus_node_info_new_for_xml("<node><interface name='org.freedesktop.portal.Request'><method name='Close'/><signal name='Response'><arg type='u'/><arg type='a{sv}'/></signal></interface></node>", &error);
  if (!node) { fprintf(stderr, "%s\n", error->message); return 1; }
  GDBusNodeInfo *portal_node = g_dbus_node_info_new_for_xml(portal_xml, &error);
  if (!portal_node) { fprintf(stderr, "%s\n", error->message); return 1; }
  connection = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
  if (!connection) { fprintf(stderr, "%s\n", error->message); return 1; }
  GDBusInterfaceInfo *portal_info = g_dbus_node_info_lookup_interface(portal_node, "org.freedesktop.portal.FileChooser");
  guint registration = g_dbus_connection_register_object(connection,
      "/org/freedesktop/portal/desktop", portal_info, &portal_vtable, NULL, NULL, &error);
  if (!registration) { fprintf(stderr, "%s\n", error->message); return 1; }
  g_bus_own_name_on_connection(connection, "org.freedesktop.portal.Desktop",
      G_BUS_NAME_OWNER_FLAGS_DO_NOT_QUEUE, name_acquired, NULL, NULL, NULL);
  loop = g_main_loop_new(NULL, FALSE);
  g_main_loop_run(loop);
  return 0;
}
