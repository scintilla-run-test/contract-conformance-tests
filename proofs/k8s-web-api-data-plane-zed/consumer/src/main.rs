fn main() {
    assert_eq!(k8s_web_api_data_plane::CONTRACT, "k8s-web-api-data-plane/v1");
    assert_eq!(k8s_web_api_data_plane::modes().len(), 4);
}
