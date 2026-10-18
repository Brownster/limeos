use super::*;

pub(super) async fn enable<B: Backend>(State(app): State<App<B>>, request: Request) -> Response {
    // Trusted process-local budget; JSON never supplies or renews this value.
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = async {
        let (token, csrf) = operations::credentials(&app, request.headers())?;
        let ip = request
            .extensions()
            .get::<Peer>()
            .map(|p| p.0.ip())
            .unwrap_or(IpAddr::from([127, 0, 0, 1]));
        if !app.limiter.lock().is_ok_and(|mut l| l.allow(ip)) {
            return Err(Error(ErrorCode::Overloaded));
        }
        let input: limeos_contracts::ObserveHealthEnable = operations::input(request).await?;
        if input.password.is_empty() || input.password.len() > 1024 {
            return Err(Error(ErrorCode::InvalidInput));
        }
        app.backend
            .enable_observe_health(token, csrf, input, deadline)
            .await
    }
    .await;
    match result {
        Ok(value) if value.reauthenticate => (
            [(
                "set-cookie",
                "__Host-limeos=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0",
            )],
            axum::Json(value),
        )
            .into_response(),
        Ok(value) => axum::Json(value).into_response(),
        Err(error) => failure(error),
    }
}

#[cfg(test)]
mod tests;
