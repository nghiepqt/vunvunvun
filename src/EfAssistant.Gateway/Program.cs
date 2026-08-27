var builder = WebApplication.CreateBuilder(args);
var app = builder.Build();

app.MapGet("/", () => "EfAssistant Gateway API");

app.Run();
