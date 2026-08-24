package corpus.module.internal;

import corpus.module.api.Service;

public final class ServiceImpl implements Service {
    @Override
    public String name() {
        return "service";
    }
}
