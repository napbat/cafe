module corpus.module {
    exports corpus.module.api;
    uses corpus.module.api.Service;
    provides corpus.module.api.Service with corpus.module.internal.ServiceImpl;
}
